use color8::palette::{ColorBlend, color_from_palette16};
use color8::presets::RAINBOW_COLORS;
use color8::rgb::Crgb;
use color8::{Chsv, fade_to_black_by, nscale8};
use defmt::info;
use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Ticker};
use esp_hal_smartled::{SmartLedsAdapter, buffer_size};
use lib8tion::{Fract8, beat8, cos16, scale8, sin16, triwave8};
use smart_leds::{RGB8, SmartLedsWrite as _};

use crate::layout3d::{CubeFaces, Layout3d};
use crate::vec3::Vec3;

pub const NUM_LEDS: usize = 150;

// `led_task` is the sole owner of `LedControl`; other tasks (including `ota_task`, in a
// separate crate module) send it commands over this channel instead of sharing it behind
// a mutex.
pub enum LedCommand {
    NextEffect,
    SetEnabled(bool),
    ShowWifiConnecting,
    ShowWifiConnected,
    ShowWifiOff,
    ShowOtaInProgress,
    ShowOtaSuccess,
    ResumeAnimation,
}

pub static LED_CHANNEL: Channel<CriticalSectionRawMutex, LedCommand, 4> = Channel::new();

// Hoisted out of the generic impl below: rustc rejects associated consts in
// a `[T; N]` array-length position inside `impl<const BUFFER_SIZE: usize>`.
//
// Uses the *blocking* adapter's sizing/API rather than the async one: the async adapter
// transmits one LED at a time as separate awaited RMT operations, leaving the line idle
// between each `.await` point for however long the executor takes to resume it. With Wi-Fi
// tasks in the mix that gap can stretch past the WS2812 reset threshold and corrupt the
// display. The blocking adapter issues a single continuous RMT transmission for the whole
// frame, so there's no such gap (a `led_task` iteration blocks briefly, but that's fine).
pub const LED_BUFFER_SIZE: usize = buffer_size(NUM_LEDS);
pub type SharedLedControl = LedControl<'static, LED_BUFFER_SIZE>;

const COLS: usize = 5;
const LEVEL: u8 = 30;

// Shared max brightness applied by every effect.
const BRIGHTNESS: u8 = 45;

// Larson scanner ("Knight Rider") tuning.
const SCANNER_COLOR: RGB8 = RGB8 { r: 255, g: 0, b: 0 };
const SCANNER_FADE_BY: u8 = 64;

// Rainbow-cube effect tuning. Chsv::from_hue takes a `u8`, so the hue phase is plain
// wrapping integer math — no need for floating-point rem_euclid or a num_traits dependency.
const RAINBOW_POS_SCALE: f32 = 0.75;
const RAINBOW_MILLIS_PER_STEP: u64 = 40;

// Pulsating effect
const PULSE_BPM: u16 = 15;

pub const BLACK: RGB8 = RGB8 { r: 0, g: 0, b: 0 };
pub const RED: RGB8 = RGB8 {
    r: LEVEL,
    g: 0,
    b: 0,
};
pub const GREEN: RGB8 = RGB8 {
    r: 0,
    g: LEVEL,
    b: 0,
};
pub const BLUE: RGB8 = RGB8 {
    r: 0,
    g: 0,
    b: LEVEL,
};
pub const YELLOW: RGB8 = RGB8 {
    r: LEVEL,
    g: LEVEL,
    b: 0,
};
pub const MAGENTA: RGB8 = RGB8 {
    r: LEVEL,
    g: 0,
    b: LEVEL,
};
pub const CYAN: RGB8 = RGB8 {
    r: 0,
    g: LEVEL,
    b: LEVEL,
};
pub const WHITE: RGB8 = RGB8 {
    r: LEVEL,
    g: LEVEL,
    b: LEVEL,
};

// Local extension trait: neither `Crgb` (color8) nor `RGB8` (rgb crate) is defined in this
// crate, so a direct `impl From<Crgb> for RGB8` would violate the orphan rule. Defining our
// own trait sidesteps that, since only the trait itself needs to be local.
trait ToRgb8 {
    fn to_rgb8(self) -> RGB8;
}

impl ToRgb8 for Crgb {
    fn to_rgb8(self) -> RGB8 {
        RGB8::new(self.r, self.g, self.b)
    }
}

fn meander_index(logical: usize) -> usize {
    let row = logical / COLS;
    let col = logical % COLS;
    if row % 2 == 0 {
        logical
    } else {
        row * COLS + (COLS - 1 - col)
    }
}

/// A triangular pulse: the first half of the beat cycle ramps linearly `0..=highest..=0`
/// (via `triwave8`), the second half stays flat at 0 — a straight-edged analog of
/// `beatsin8`'s clamped-lower-half sine, snapping off for half its cycle instead of
/// following a curve.
fn beattri8_clamped(bpm: u16, highest: u8, phase_offset: u8, now_millis: u32) -> u8 {
    let beat = beat8(bpm, 0, now_millis).wrapping_add(phase_offset);
    if beat >= 128 {
        return 0;
    }
    scale8(triwave8(beat * 2), Fract8(highest))
}

pub struct LedControl<'a, const BUFFER_SIZE: usize> {
    leds: SmartLedsAdapter<'a, BUFFER_SIZE>,
    led_colors: [RGB8; NUM_LEDS],
    scanner_trail: [Crgb; NUM_LEDS],
    scanner_pos: usize,
    scanner_forward: bool,
    palette_time_index: u8,
    layout: CubeFaces,
}

impl<'a, const BUFFER_SIZE: usize> LedControl<'a, BUFFER_SIZE> {
    pub fn new(mut leds: SmartLedsAdapter<'a, BUFFER_SIZE>) -> Self {
        let led_colors = [RGB8::default(); NUM_LEDS];
        leds.write(led_colors.iter().copied()).unwrap();
        Self {
            leds,
            led_colors,
            scanner_trail: [Crgb::new(0, 0, 0); NUM_LEDS],
            scanner_pos: 0,
            scanner_forward: true,
            palette_time_index: 0,
            layout: CubeFaces::new(),
        }
    }

    pub fn fill(&mut self, color: RGB8) {
        self.leds
            .write(core::iter::repeat_n(color, NUM_LEDS))
            .unwrap();
    }

    /// Renders one frame of a Larson-scanner ("Knight Rider") sweep: a bright dot bounces
    /// end to end across the strip, leaving a fading trail. The dot advances by one LED per
    /// call, so the sweep speed is entirely up to how often the caller calls this.
    pub fn larson_scanner_frame(&mut self) {
        fade_to_black_by(&mut self.scanner_trail, SCANNER_FADE_BY);

        self.scanner_trail[meander_index(self.scanner_pos)] =
            Crgb::new(SCANNER_COLOR.r, SCANNER_COLOR.g, SCANNER_COLOR.b);

        if self.scanner_forward {
            if self.scanner_pos == NUM_LEDS - 1 {
                self.scanner_forward = false;
                self.scanner_pos -= 1;
            } else {
                self.scanner_pos += 1;
            }
        } else if self.scanner_pos == 0 {
            self.scanner_forward = true;
            self.scanner_pos += 1;
        } else {
            self.scanner_pos -= 1;
        }

        // Apply global brightness to a copy so the persistent trail buffer stays full-scale.
        let mut frame = self.scanner_trail;
        nscale8(&mut frame, BRIGHTNESS);

        for (dst, src) in self.led_colors.iter_mut().zip(frame.iter()) {
            *dst = src.to_rgb8();
        }
        self.leds.write(self.led_colors.iter().copied()).unwrap();
    }

    /// Renders one frame of a palette-cycling effect (FastLED's `ColorFromPalette`): each
    /// pixel's color comes from `RAINBOW_COLORS`, indexed by its spatial position plus a
    /// time index that advances by one every call, both wrapping around at 256.
    pub fn palette_frame(&mut self) {
        let mut spatial_index: u8 = self.palette_time_index;
        for i in 0..NUM_LEDS {
            let color = color_from_palette16(
                &RAINBOW_COLORS,
                spatial_index,
                BRIGHTNESS,
                ColorBlend::LinearBlend,
            );
            self.led_colors[i] = color.to_rgb8();
            spatial_index = spatial_index.wrapping_add(1);
        }
        self.palette_time_index = self.palette_time_index.wrapping_add(1);

        self.leds.write(self.led_colors.iter().copied()).unwrap();
    }

    pub fn rainbow_frame(&mut self, millis: u64) {
        let time_hue = (millis / RAINBOW_MILLIS_PER_STEP) as u8;

        // lib8tion's sin16/cos16 take a `u16` spanning a full circle, so truncating `millis`
        // to `u16` is itself a wrapping rotation phase — same trick as `time_hue`'s `as u8`.
        let theta = millis as u16;
        let sin_t = sin16(theta) as f32 / 32768.0;
        let cos_t = cos16(theta) as f32 / 32768.0;
        let dir = Vec3::new(
            -0.71 * cos_t + 0.41 * sin_t,
            0.71 * cos_t + 0.41 * sin_t,
            -0.82 * sin_t,
        );

        self.leds
            .write(self.layout.points().map(move |point| {
                let pos_hue = (point.dot(dir) * RAINBOW_POS_SCALE) as i32 as u8;
                let color =
                    Crgb::from(Chsv::from_hue(pos_hue.wrapping_add(time_hue))).scale8(BRIGHTNESS);
                color.to_rgb8()
            }))
            .unwrap();
    }

    pub fn pulse_frame(&mut self, millis: u64) {
        self.fill(
            Crgb {
                r: beattri8_clamped(PULSE_BPM, BRIGHTNESS, 0, millis as u32),
                g: 0, //beattri8_clamped(PULSE_BPM, BRIGHTNESS, 85, millis as u32),
                b: 0, //beattri8_clamped(PULSE_BPM, BRIGHTNESS, 170, millis as u32),
            }
            .to_rgb8(),
        );
    }
}

const FRAME_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, PartialEq)]
enum Effect {
    Palette,
    Rainbow,
    Pulse,
    LarsonScanner,
    Off,
}

impl Effect {
    fn next(self) -> Self {
        match self {
            Effect::Palette => Effect::Rainbow,
            Effect::Rainbow => Effect::Pulse,
            Effect::Pulse => Effect::LarsonScanner,
            Effect::LarsonScanner => Effect::Off,
            Effect::Off => Effect::Palette,
        }
    }
}

#[embassy_executor::task]
pub async fn led_task(mut led_control: SharedLedControl) {
    let mut ticker = Ticker::every(FRAME_INTERVAL);
    let mut effect = Effect::Rainbow;
    let mut enabled = true;
    // Set while showing Wi-Fi connection status, pausing normal effect rendering until
    // the next `NextEffect` command.
    let mut overridden = false;
    loop {
        // While running, keep rendering frames but stay ready to drop out the moment
        // another command (next effect, activity change, ...) comes in.
        let running = enabled && effect != Effect::Off && !overridden;
        let command = if running {
            match select(LED_CHANNEL.receive(), ticker.next()).await {
                Either::First(command) => command,
                Either::Second(()) => {
                    match effect {
                        Effect::Palette => led_control.palette_frame(),
                        Effect::Rainbow => {
                            led_control.rainbow_frame(embassy_time::Instant::now().as_millis())
                        }
                        Effect::Pulse => {
                            led_control.pulse_frame(embassy_time::Instant::now().as_millis())
                        }
                        Effect::LarsonScanner => led_control.larson_scanner_frame(),
                        Effect::Off => {}
                    }
                    continue;
                }
            }
        } else {
            LED_CHANNEL.receive().await
        };

        match command {
            LedCommand::NextEffect => {
                overridden = false;
                effect = effect.next();
                match effect {
                    Effect::Palette => info!("Effect: palette"),
                    Effect::Rainbow => info!("Effect: rainbow"),
                    Effect::Pulse => info!("Effect: pulse"),
                    Effect::LarsonScanner => info!("Effect: larson scanner"),
                    Effect::Off => info!("Effect: off"),
                }
            }
            LedCommand::SetEnabled(new_enabled) => enabled = new_enabled,
            LedCommand::ShowWifiConnecting => {
                overridden = true;
                led_control.fill(YELLOW);
            }
            LedCommand::ShowWifiConnected => {
                overridden = true;
                led_control.fill(GREEN);
            }
            LedCommand::ShowWifiOff => {
                overridden = true;
                led_control.fill(RED);
            }
            LedCommand::ShowOtaInProgress => {
                overridden = true;
                led_control.fill(BLUE);
            }
            LedCommand::ShowOtaSuccess => {
                overridden = true;
                led_control.fill(BLACK);
            }
            LedCommand::ResumeAnimation => overridden = false,
        }

        if overridden {
            // Leave the solid status color in place until the next effect change.
        } else if enabled && effect != Effect::Off {
            // The ticker fell behind while paused and would otherwise fire in a burst to
            // catch up, making the next effect briefly run too fast.
            ticker.reset();
        } else {
            led_control.fill(BLACK);
        }
    }
}
