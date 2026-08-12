use esp_hal_smartled::SmartLedsAdapter;
use smart_leds::{RGB8, SmartLedsWrite as _};

pub const NUM_LEDS: usize = 150;

const COLS: usize = 5;
const LEVEL: u8 = 30;

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

const COLOR_WHEEL: [RGB8; 8] = [RED, GREEN, BLUE, YELLOW, MAGENTA, CYAN, WHITE, BLACK];

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
    num_leds: usize,
    color_index: usize,
}

impl<'a, const BUFFER_SIZE: usize> LedControl<'a, BUFFER_SIZE> {
    pub fn new(mut leds: SmartLedsAdapter<'a, BUFFER_SIZE>) -> Self {
        let led_colors = [RGB8::default(); NUM_LEDS];
        leds.write(led_colors.iter().copied()).unwrap();
        Self {
            leds,
            led_colors,
            num_leds: 0,
            color_index: 0,
        }
    }

    pub fn on_button_press(&mut self) {
        if self.num_leds == NUM_LEDS {
            self.num_leds = 0;
            self.color_index = (self.color_index + 1) % COLOR_WHEEL.len();
        } else {
            self.num_leds += 1;
        }
        self.update();
    }

    fn update(&mut self) {
        for i in 0..NUM_LEDS {
            let physical = meander_index(i);
            self.led_colors[physical] = if i < self.num_leds {
                COLOR_WHEEL[self.color_index]
            } else {
                BLACK
            };
        }
        self.leds.write(self.led_colors.iter().copied()).unwrap();
    }

    pub fn fill(&mut self, color: RGB8) {
        self.leds.write([color; NUM_LEDS].into_iter()).unwrap();
    }

    /// Fills the cube from accelerometer axis readings (x→R, y→G, z→B).
    pub fn set_from_acceleration(&mut self, x: i16, y: i16, z: i16) {
        let color = RGB8 {
            r: axis_to_channel(x),
            g: axis_to_channel(y),
            b: axis_to_channel(z),
        };
        self.fill(color);
    }
}

// ADXL362 12-bit reading range at +/-2g is approximately -2048..=2047.
const ACCEL_MAX_MAGNITUDE: u16 = 2048;

fn axis_to_channel(value: i16) -> u8 {
    let magnitude = value.unsigned_abs().min(ACCEL_MAX_MAGNITUDE);
    (magnitude as u32 * LEVEL as u32 / ACCEL_MAX_MAGNITUDE as u32) as u8
}
