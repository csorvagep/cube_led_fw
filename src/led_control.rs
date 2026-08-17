use color8::rgb::Crgb;
use color8::{fade_to_black_by, nscale8};
use esp_hal_smartled::{SmartLedsAdapterAsync, buffer_size_async};
use smart_leds::{RGB8, SmartLedsWriteAsync as _};

pub const NUM_LEDS: usize = 150;

// Hoisted out of the generic impl below: rustc rejects associated consts in
// a `[T; N]` array-length position inside `impl<const BUFFER_SIZE: usize>`.
pub const LED_BUFFER_SIZE: usize = buffer_size_async(NUM_LEDS);
pub type SharedLedControl = LedControl<'static, LED_BUFFER_SIZE>;

const COLS: usize = 5;
const LEVEL: u8 = 30;

// Larson scanner ("Knight Rider") tuning.
const SCANNER_COLOR: RGB8 = RGB8 { r: 255, g: 0, b: 0 };
const SCANNER_BRIGHTNESS: u8 = 60;
const SCANNER_FADE_BY: u8 = 64;

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
    leds: SmartLedsAdapterAsync<'a, BUFFER_SIZE>,
    led_colors: [RGB8; NUM_LEDS],
    scanner_trail: [Crgb; NUM_LEDS],
    scanner_pos: usize,
    scanner_forward: bool,
}

impl<'a, const BUFFER_SIZE: usize> LedControl<'a, BUFFER_SIZE> {
    pub async fn new(mut leds: SmartLedsAdapterAsync<'a, BUFFER_SIZE>) -> Self {
        let led_colors = [RGB8::default(); NUM_LEDS];
        leds.write(led_colors.iter().copied()).await.unwrap();
        Self {
            leds,
            led_colors,
            scanner_trail: [Crgb::new(0, 0, 0); NUM_LEDS],
            scanner_pos: 0,
            scanner_forward: true,
        }
    }

    pub async fn fill(&mut self, color: RGB8) {
        self.leds
            .write(core::iter::repeat_n(color, NUM_LEDS))
            .await
            .unwrap();
    }

    /// Renders one frame of a Larson-scanner ("Knight Rider") sweep: a bright dot bounces
    /// end to end across the strip, leaving a fading trail. The dot advances by one LED per
    /// call, so the sweep speed is entirely up to how often the caller calls this.
    pub async fn larson_scanner_frame(&mut self) {
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
        nscale8(&mut frame, SCANNER_BRIGHTNESS);

        for (dst, src) in self.led_colors.iter_mut().zip(frame.iter()) {
            *dst = RGB8 {
                r: src.r,
                g: src.g,
                b: src.b,
            };
        }
        self.leds
            .write(self.led_colors.iter().copied())
            .await
            .unwrap();
    }
}
