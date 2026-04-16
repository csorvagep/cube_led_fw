#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use cubeled::led_control::{LedControl, NUM_LEDS};
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, InputConfig, Output, OutputConfig, Pull};
use esp_hal::main;
use esp_hal::rmt::Rmt;
use esp_hal::time::Rate;
use esp_hal_smartled::{SmartLedsAdapter, smart_led_buffer};
use {esp_backtrace as _, esp_println as _};

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let _peripherals = esp_hal::init(config);

    // LD_ON pin
    let mut ld_on = Output::new(
        _peripherals.GPIO7,
        esp_hal::gpio::Level::Low,
        OutputConfig::default(),
    );
    ld_on.set_low();

    // BTN
    let btn = Input::new(
        _peripherals.GPIO9,
        InputConfig::default().with_pull(Pull::Up),
    );

    // LED driver
    let rmt = Rmt::new(_peripherals.RMT, Rate::from_mhz(80)).unwrap();
    let mut led_buffer = smart_led_buffer!(NUM_LEDS);
    let leds = SmartLedsAdapter::new(rmt.channel0, _peripherals.GPIO8, &mut led_buffer);
    let mut led_control = LedControl::new(leds);

    let delay = Delay::new();

    let mut button_state = btn.is_high();

    loop {
        let current_btn_state = btn.is_high();
        if button_state != current_btn_state {
            if !current_btn_state {
                led_control.on_button_press();
            }
        }
        button_state = current_btn_state;

        delay.delay_millis(100);
    }
}
