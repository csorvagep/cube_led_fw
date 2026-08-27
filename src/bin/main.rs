#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use core::net::Ipv4Addr;

use cubeled::adxl362::{
    ActivityConfig, Adxl362, InactivityConfig, InterruptConfig, NoiseMode, OutputDataRate, Range,
};
use cubeled::led_control::{BLACK, LED_BUFFER_SIZE, LedControl, SharedLedControl, YELLOW};
use defmt::info;
use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_net::{ConfigV4, Ipv4Cidr, Runner, StackResources, StaticConfigV4};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::{Duration, Ticker, Timer, with_timeout};
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, InputConfig, Output, OutputConfig, Pull};
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::rmt::{PulseCode, Rmt};
use esp_hal::rng::Rng;
use esp_hal::spi::master::{Config, Spi};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hal_smartled::SmartLedsAdapterAsync;
use esp_radio::Controller;
use esp_radio::wifi::{
    ClientConfig, ModeConfig, WifiController, WifiDevice, WifiEvent, WifiStaState,
};
use static_cell::StaticCell;
use {esp_backtrace as _, esp_println as _};

#[path = "secrets/wifi_secrets.rs"]
mod wifi_secrets;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

// SPI stays blocking: ADXL362 transfers are tiny (3-8 bytes), not worth a DMA+async rewrite.
type Accel = Adxl362<Spi<'static, esp_hal::Blocking>>;

// `LedControl` wraps an `esp_hal::Async` RMT channel, which is `!Send`, so it can't live
// behind a shared `static Mutex`. Instead `led_task` is its sole owner and the other tasks
// send it commands over this channel.
enum LedCommand {
    NextEffect,
    SetEnabled(bool),
}

static LED_CHANNEL: Channel<CriticalSectionRawMutex, LedCommand, 4> = Channel::new();
static LED_BUF: StaticCell<[PulseCode; LED_BUFFER_SIZE]> = StaticCell::new();

static ESP_RADIO_CTRL: StaticCell<Controller<'static>> = StaticCell::new();
static NET_RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();

// Used only if no DHCP lease shows up within `DHCP_TIMEOUT` (e.g. an AP with no DHCP server).
const STATIC_IP: Ipv4Addr = Ipv4Addr::new(192, 168, 137, 50);
const STATIC_GATEWAY: Ipv4Addr = Ipv4Addr::new(192, 168, 137, 1);
const DHCP_TIMEOUT: Duration = Duration::from_secs(10);

// A module boundary is needed here: an `#[allow(clippy::large_stack_frames)]` placed
// directly on an `#[embassy_executor::task]` fn doesn't reach the coroutine clippy
// actually measures, since the task macro's expansion doesn't forward it that deep.
mod led_task_mod {
    #![allow(
        clippy::large_stack_frames,
        reason = "led_task owns the full LED color buffer for its whole lifetime"
    )]

    use defmt::info;
    use embassy_futures::select::{Either, select};
    use embassy_time::{Duration, Ticker};

    use crate::led_task_mod::Effect::LarsonScanner;

    use super::{LedCommand, SharedLedControl};

    const FRAME_INTERVAL: Duration = Duration::from_millis(50);

    #[derive(Clone, Copy, PartialEq)]
    enum Effect {
        Palette,
        Rainbow,
        LarsonScanner,
        Off,
    }

    impl Effect {
        fn next(self) -> Self {
            match self {
                Effect::Palette => Effect::Rainbow,
                Effect::Rainbow => Effect::LarsonScanner,
                Effect::LarsonScanner => Effect::Off,
                Effect::Off => Effect::Palette,
            }
        }
    }

    #[embassy_executor::task]
    pub async fn led_task(mut led_control: SharedLedControl) {
        let mut ticker = Ticker::every(FRAME_INTERVAL);
        let mut effect = Effect::Palette;
        let mut enabled = true;
        loop {
            // While running, keep rendering frames but stay ready to drop out the moment
            // another command (next effect, activity change, ...) comes in.
            let running = enabled && effect != Effect::Off;
            let command = if running {
                match select(super::LED_CHANNEL.receive(), ticker.next()).await {
                    Either::First(command) => command,
                    Either::Second(()) => {
                        match effect {
                            Effect::Palette => led_control.palette_frame().await,
                            Effect::Rainbow => {
                                led_control
                                    .rainbow_frame(embassy_time::Instant::now().as_millis())
                                    .await
                            }
                            Effect::LarsonScanner => led_control.larson_scanner_frame().await,
                            Effect::Off => {}
                        }
                        continue;
                    }
                }
            } else {
                super::LED_CHANNEL.receive().await
            };

            match command {
                LedCommand::NextEffect => {
                    effect = effect.next();
                    match effect {
                        Effect::Palette => info!("Effect: palette"),
                        Effect::Rainbow => info!("Effect: rainbow"),
                        Effect::LarsonScanner => info!("Effect: larson scanner"),
                        Effect::Off => info!("Effect: off"),
                    }
                }
                LedCommand::SetEnabled(new_enabled) => enabled = new_enabled,
            }

            if enabled && effect != Effect::Off {
                // The ticker fell behind while paused and would otherwise fire in a burst to
                // catch up, making the next effect briefly run too fast.
                ticker.reset();
            } else {
                led_control.fill(super::BLACK).await;
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
            LED_CHANNEL.send(LedCommand::NextEffect).await;
        }
    }
}

#[embassy_executor::task]
async fn sensor_task(mut adxl_int2: Input<'static>, mut accel: Accel) {
    // Number of 100ms ticks of continued inactivity before the LEDs turn off.
    const INACTIVITY_OFF_TICKS: u32 = 10_000 / 100;
    let mut inactivity_ticks: Option<u32> = None;
    let mut ticker = Ticker::every(Duration::from_millis(100));

    loop {
        match select(adxl_int2.wait_for_rising_edge(), ticker.next()).await {
            Either::First(()) => {
                let status = accel.read_status().expect("Failed to read ADXL362 status");
                if status.act {
                    info!("Activity detected");
                    inactivity_ticks = None;
                    LED_CHANNEL.send(LedCommand::SetEnabled(true)).await;
                }
                if status.inact {
                    info!("Inactivity detected");
                    inactivity_ticks = Some(INACTIVITY_OFF_TICKS);
                }
            }
            Either::Second(()) => {
                if let Some(ticks) = inactivity_ticks {
                    if ticks == 0 {
                        inactivity_ticks = None;
                        LED_CHANNEL.send(LedCommand::SetEnabled(false)).await;
                    } else {
                        inactivity_ticks = Some(ticks - 1);
                    }
                }
            }
        }
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    runner.run().await
}

#[embassy_executor::task]
async fn wifi_task(mut controller: WifiController<'static>) {
    loop {
        if esp_radio::wifi::sta_state() == WifiStaState::Connected {
            // Already connected; wait here until we drop off before trying again.
            controller.wait_for_event(WifiEvent::StaDisconnected).await;
            info!("Wifi disconnected");
            Timer::after(Duration::from_secs(5)).await;
        }

        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = ModeConfig::Client(
                ClientConfig::default()
                    .with_ssid(wifi_secrets::SSID.into())
                    .with_password(wifi_secrets::PASSWORD.into()),
            );
            controller.set_config(&client_config).unwrap();
            info!("Starting wifi");
            controller.start_async().await.unwrap();
        }

        info!("Connecting to wifi...");
        match controller.connect_async().await {
            Ok(()) => info!("Wifi connected"),
            Err(_) => {
                info!("Failed to connect to wifi, retrying");
                Timer::after(Duration::from_secs(5)).await;
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

    esp_alloc::heap_allocator!(size: 64 * 1024);

    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    let esp_radio_ctrl: &'static Controller<'static> =
        &*ESP_RADIO_CTRL.init(esp_radio::init().unwrap());
    let (wifi_controller, wifi_interfaces) =
        esp_radio::wifi::new(esp_radio_ctrl, peripherals.WIFI, Default::default()).unwrap();

    let net_seed = {
        let rng = Rng::new();
        (rng.random() as u64) << 32 | rng.random() as u64
    };
    let (net_stack, net_runner) = embassy_net::new(
        wifi_interfaces.sta,
        embassy_net::Config::dhcpv4(Default::default()),
        NET_RESOURCES.init(StackResources::new()),
        net_seed,
    );

    spawner.spawn(net_task(net_runner)).unwrap();
    spawner.spawn(wifi_task(wifi_controller)).unwrap();

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

    info!("Waiting for wifi link...");
    while !net_stack.is_link_up() {
        Timer::after(Duration::from_millis(500)).await;
    }

    info!("Waiting for IP address (DHCP)...");
    let dhcp_config = with_timeout(DHCP_TIMEOUT, async {
        loop {
            if let Some(config) = net_stack.config_v4() {
                return config;
            }
            Timer::after(Duration::from_millis(500)).await;
        }
    })
    .await;
    match dhcp_config {
        Ok(config) => info!("Got IP via DHCP: {}", config.address),
        Err(_) => {
            info!("No DHCP lease after {}s, falling back to static IP", DHCP_TIMEOUT.as_secs());
            net_stack.set_config_v4(ConfigV4::Static(StaticConfigV4 {
                address: Ipv4Cidr::new(STATIC_IP, 24),
                gateway: Some(STATIC_GATEWAY),
                dns_servers: Default::default(),
            }));
            info!("Using static IP: {}", STATIC_IP);
        }
    }

    loop {
        Timer::after(Duration::from_secs(3600)).await;
    }
}
