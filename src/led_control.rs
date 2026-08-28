use color8::palette::{ColorBlend, color_from_palette16};
use color8::presets::RAINBOW_COLORS;
use color8::rgb::Crgb;
use color8::{Chsv, fade_to_black_by, nscale8};
use esp_hal_smartled::{SmartLedsAdapter, buffer_size};
use lib8tion::{cos16, sin16};
use smart_leds::{RGB8, SmartLedsWrite as _};

use crate::layout3d::{CubeFaces, Layout3d};
use crate::vec3::Vec3;

pub const NUM_LEDS: usize = 150;

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
const BRIGHTNESS: u8 = 60;

// Larson scanner ("Knight Rider") tuning.
const SCANNER_COLOR: RGB8 = RGB8 { r: 255, g: 0, b: 0 };
const SCANNER_FADE_BY: u8 = 64;

// Rainbow-cube effect tuning. Chsv::from_hue takes a `u8`, so the hue phase is plain
// wrapping integer math — no need for floating-point rem_euclid or a num_traits dependency.
const RAINBOW_POS_SCALE: f32 = 0.75;
const RAINBOW_MILLIS_PER_STEP: u64 = 40;

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

fn meander_index(logical: usize) -> usize {
    let row = logical / COLS;
    let col = logical % COLS;
    if row % 2 == 0 {
        logical
    } else {
        row * COLS + (COLS - 1 - col)
    }
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
            *dst = RGB8 {
                r: src.r,
                g: src.g,
                b: src.b,
            };
        }
        self.leds
            .write(self.led_colors.iter().copied())
            .unwrap();
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
            self.led_colors[i] = RGB8 {
                r: color.r,
                g: color.g,
                b: color.b,
            };
            spatial_index = spatial_index.wrapping_add(1);
        }
        self.palette_time_index = self.palette_time_index.wrapping_add(1);

        self.leds
            .write(self.led_colors.iter().copied())
            .unwrap();
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
                RGB8 {
                    r: color.r,
                    g: color.g,
                    b: color.b,
                }
            }))
            .unwrap();
    }
}
