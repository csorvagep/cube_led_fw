#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use cubeled::adxl362::{
    ActivityConfig, Adxl362, InactivityConfig, InterruptConfig, NoiseMode, OutputDataRate, Range,
};
use cubeled::led_control::{BLACK, LedControl, NUM_LEDS, YELLOW};
use defmt::info;
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, InputConfig, Output, OutputConfig, Pull};
use esp_hal::main;
use esp_hal::rmt::Rmt;
use esp_hal::spi::master::{Config, Spi};
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
    let peripherals = esp_hal::init(config);

    // LD_ON pin
    let mut ld_on = Output::new(
        peripherals.GPIO7,
        esp_hal::gpio::Level::Low,
        OutputConfig::default(),
    );
    ld_on.set_low();

    // BTN
    let btn = Input::new(
        peripherals.GPIO9,
        InputConfig::default().with_pull(Pull::Up),
    );

    // ADXL362 INT2 (activity/inactivity)
    let adxl_int2 = Input::new(peripherals.GPIO4, InputConfig::default());

    // LED driver
    let rmt = Rmt::new(peripherals.RMT, Rate::from_mhz(80)).unwrap();
    let mut led_buffer = smart_led_buffer!(NUM_LEDS);
    let leds = SmartLedsAdapter::new(rmt.channel0, peripherals.GPIO8, &mut led_buffer);
    let mut led_control = LedControl::new(leds);

    let mut delay = Delay::new();

    let mut button_state = btn.is_high();

    info!("CubeLED started");
    info!("Filling cube with color YELLOW");
    led_control.fill(YELLOW);

    // SPI
    let mosi = peripherals.GPIO3;
    let miso = peripherals.GPIO1;
    let sck = peripherals.GPIO0;
    let cs = peripherals.GPIO10;

    let spi = Spi::new(peripherals.SPI2, Config::default())
        .expect("Failed to acquire SPI2")
        .with_mosi(mosi)
        .with_miso(miso)
        .with_sck(sck)
        .with_cs(cs);

    let mut accel = Adxl362::new(spi);
    accel
        .init(&mut delay)
        .expect("Failed to initialize ADXL362");
    accel
        .configure_filter(OutputDataRate::Hz100, Range::G2, NoiseMode::Normal)
        .expect("Failed to configure ADXL362 filter");
    accel
        .configure_activity(&ActivityConfig {
            threshold: 200, // ~0.2g at +/-2g range
            time: 3,
            referenced: true,
        })
        .expect("Failed to configure ADXL362 activity detection");
    accel
        .configure_inactivity(&InactivityConfig {
            threshold: 50, // ~0.05g at +/-2g range
            time: 300,     // ~3s at 100Hz ODR
            referenced: true,
        })
        .expect("Failed to configure ADXL362 inactivity detection");
    accel
        .enable_linked_mode()
        .expect("Failed to enable ADXL362 linked mode");
    accel
        .configure_interrupt2(&InterruptConfig {
            active_low: false,
            awake: false,
            act: true,
            inact: true,
            data_ready: false,
        })
        .expect("Failed to configure ADXL362 INT2 mapping");
    accel
        .set_active(true)
        .expect("Failed to start ADXL362 measurement");

    // Number of 100ms loop iterations the LEDs stay off after inactivity is detected.
    const INACTIVITY_OFF_TICKS: u32 = 10_000 / 100;
    let mut leds_off = false;
    let mut inactivity_ticks: Option<u32> = None;

    loop {
        let current_btn_state = btn.is_high();
        if button_state != current_btn_state {
            if !current_btn_state {
                led_control.on_button_press();
                info!("Button pressed");
            }
        }
        button_state = current_btn_state;

        if adxl_int2.is_high() {
            let status = accel.read_status().expect("Failed to read ADXL362 status");
            if status.act {
                info!("Activity detected");
                leds_off = false;
                inactivity_ticks = None;
            }
            if status.inact {
                info!("Inactivity detected");
                inactivity_ticks = Some(INACTIVITY_OFF_TICKS);
            }
        }

        if let Some(ticks) = inactivity_ticks {
            if ticks == 0 {
                leds_off = true;
                inactivity_ticks = None;
            } else {
                inactivity_ticks = Some(ticks - 1);
            }
        }

        if leds_off {
            led_control.fill(BLACK);
        } else {
            let acceleration = accel
                .read_acceleration()
                .expect("Failed to read acceleration");
            info!(
                "Acceleration: x={} y={} z={}",
                acceleration.x, acceleration.y, acceleration.z
            );
            led_control.set_from_acceleration(acceleration.x, acceleration.y, acceleration.z);
        }

        delay.delay_millis(100);
    }
}
