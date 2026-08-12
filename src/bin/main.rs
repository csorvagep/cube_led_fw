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
use cubeled::led_control::{
    BLACK, LED_BUFFER_SIZE, LedControl, SharedLedControl, YELLOW, acceleration_to_color,
};
use defmt::info;
use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Ticker, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, InputConfig, Output, OutputConfig, Pull};
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::rmt::{PulseCode, Rmt};
use esp_hal::spi::master::{Config, Spi};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hal_smartled::SmartLedsAdapterAsync;
use smart_leds::RGB8;
use static_cell::StaticCell;
use {esp_backtrace as _, esp_println as _};

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

// SPI stays blocking: ADXL362 transfers are tiny (3-8 bytes), not worth a DMA+async rewrite.
type Accel = Adxl362<Spi<'static, esp_hal::Blocking>>;

// `LedControl` wraps an `esp_hal::Async` RMT channel, which is `!Send`, so it can't live
// behind a shared `static Mutex`. Instead `led_task` is its sole owner and the other tasks
// send it commands over this channel.
enum LedCommand {
    ButtonPress,
    Fill(RGB8),
}

static LED_CHANNEL: Channel<CriticalSectionRawMutex, LedCommand, 4> = Channel::new();
static LED_BUF: StaticCell<[PulseCode; LED_BUFFER_SIZE]> = StaticCell::new();

// A module boundary is needed here: an `#[allow(clippy::large_stack_frames)]` placed
// directly on an `#[embassy_executor::task]` fn doesn't reach the coroutine clippy
// actually measures, since the task macro's expansion doesn't forward it that deep.
mod led_task_mod {
    #![allow(
        clippy::large_stack_frames,
        reason = "led_task owns the full LED color buffer for its whole lifetime"
    )]

    use defmt::info;

    use super::{LedCommand, SharedLedControl};

    #[embassy_executor::task]
    pub async fn led_task(mut led_control: SharedLedControl) {
        loop {
            match super::LED_CHANNEL.receive().await {
                LedCommand::ButtonPress => {
                    led_control.on_button_press().await;
                    info!("Button pressed");
                }
                LedCommand::Fill(color) => led_control.fill(color).await,
            }
        }
    }
}
use led_task_mod::led_task;

#[embassy_executor::task]
async fn button_task(mut btn: Input<'static>) {
    loop {
        btn.wait_for_any_edge().await;
        if !btn.is_high() {
            LED_CHANNEL.send(LedCommand::ButtonPress).await;
        }
    }
}

#[embassy_executor::task]
async fn sensor_task(mut adxl_int2: Input<'static>, mut accel: Accel) {
    // Number of 100ms ticks the LEDs stay off after inactivity is detected.
    const INACTIVITY_OFF_TICKS: u32 = 10_000 / 100;
    let mut leds_off = false;
    let mut inactivity_ticks: Option<u32> = None;
    let mut ticker = Ticker::every(Duration::from_millis(100));

    loop {
        match select(adxl_int2.wait_for_rising_edge(), ticker.next()).await {
            Either::First(()) => {
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
            Either::Second(()) => {
                if let Some(ticks) = inactivity_ticks {
                    if ticks == 0 {
                        leds_off = true;
                        inactivity_ticks = None;
                    } else {
                        inactivity_ticks = Some(ticks - 1);
                    }
                }

                let color = if leds_off {
                    BLACK
                } else {
                    let acceleration = accel
                        .read_acceleration()
                        .expect("Failed to read acceleration");
                    info!(
                        "Acceleration: x={} y={} z={}",
                        acceleration.x, acceleration.y, acceleration.z
                    );
                    acceleration_to_color(acceleration.x, acceleration.y, acceleration.z)
                };
                LED_CHANNEL.send(LedCommand::Fill(color)).await;
            }
        }
    }
}

#[allow(
    clippy::large_stack_frames,
    reason = "it's not unusual to allocate larger buffers etc. in main"
)]
#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

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
    let rmt = Rmt::new(peripherals.RMT, Rate::from_mhz(80))
        .unwrap()
        .into_async();
    let led_buffer = LED_BUF.init([PulseCode::default(); LED_BUFFER_SIZE]);
    let leds = SmartLedsAdapterAsync::new(rmt.channel0, peripherals.GPIO8, led_buffer);
    let mut led_control = LedControl::new(leds).await;

    info!("CubeLED started");
    info!("Filling cube with color YELLOW");
    led_control.fill(YELLOW).await;

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

    let mut delay = Delay::new();
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

    spawner.spawn(led_task(led_control)).unwrap();
    spawner.spawn(button_task(btn)).unwrap();
    spawner.spawn(sensor_task(adxl_int2, accel)).unwrap();

    loop {
        Timer::after(Duration::from_secs(3600)).await;
    }
}
