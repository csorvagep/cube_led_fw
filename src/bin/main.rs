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
use cubeled::led_control::{
    BLACK, GREEN, LED_BUFFER_SIZE, LedControl, RED, SharedLedControl, YELLOW,
};
use cubeled::ota::{log_booted_partition, ota_task};
use cubeled::sleep_control::{WakeReason, classify_wakeup, enter_deep_sleep};
use defmt::info;
use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_net::{ConfigV4, Ipv4Cidr, Runner, StackResources, StaticConfigV4};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Ticker, Timer, with_timeout};
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Input, InputConfig, Output, OutputConfig, Pull};
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::rmt::{PulseCode, Rmt};
use esp_hal::rng::Rng;
use esp_hal::rtc_cntl::Rtc;
use esp_hal::spi::master::{Config, Spi};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hal_smartled::SmartLedsAdapter;
use esp_radio::Controller;
use esp_radio::wifi::{ClientConfig, ModeConfig, WifiController, WifiDevice, WifiEvent};
use esp_storage::FlashStorage;
use static_cell::StaticCell;
use {esp_backtrace as _, esp_println as _};

#[path = "secrets/wifi_secrets.rs"]
mod wifi_secrets;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

// SPI stays blocking: ADXL362 transfers are tiny (3-8 bytes), not worth a DMA+async rewrite.
type Accel = Adxl362<Spi<'static, esp_hal::Blocking>>;

// `led_task` is the sole owner of `LedControl`; other tasks send it commands over this
// channel instead of sharing it behind a mutex.
enum LedCommand {
    NextEffect,
    SetEnabled(bool),
    ShowWifiConnecting,
    ShowWifiConnected,
    ShowWifiOff,
    ResumeAnimation,
}

static LED_CHANNEL: Channel<CriticalSectionRawMutex, LedCommand, 4> = Channel::new();
static LED_BUF: StaticCell<[PulseCode; LED_BUFFER_SIZE]> = StaticCell::new();

// Wi-Fi (and OTA) are off until the user holds the button for `LONG_PRESS_DURATION`, both
// to avoid the RMT/Wi-Fi interrupt contention glitch during normal operation and to save
// power — OTA updates are a rare, deliberate action, not something needed on every boot.
// The first long press brings Wi-Fi up; `wifi_task` consumes every long press after that to
// toggle it back off (and on again), so this signal has exactly one consumer at a time.
static WIFI_TOGGLE: Signal<CriticalSectionRawMutex, ()> = Signal::new();
const LONG_PRESS_DURATION: Duration = Duration::from_secs(1);

static ESP_RADIO_CTRL: StaticCell<Controller<'static>> = StaticCell::new();
static NET_RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();

// Used only if no DHCP lease shows up within `DHCP_TIMEOUT` (e.g. an AP with no DHCP server).
const STATIC_IP: Ipv4Addr = Ipv4Addr::new(192, 168, 137, 50);
const STATIC_GATEWAY: Ipv4Addr = Ipv4Addr::new(192, 168, 137, 1);
const DHCP_TIMEOUT: Duration = Duration::from_secs(30);

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
                match select(super::LED_CHANNEL.receive(), ticker.next()).await {
                    Either::First(command) => command,
                    Either::Second(()) => {
                        match effect {
                            Effect::Palette => led_control.palette_frame(),
                            Effect::Rainbow => {
                                led_control.rainbow_frame(embassy_time::Instant::now().as_millis())
                            }
                            Effect::LarsonScanner => led_control.larson_scanner_frame(),
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
                    overridden = false;
                    effect = effect.next();
                    match effect {
                        Effect::Palette => info!("Effect: palette"),
                        Effect::Rainbow => info!("Effect: rainbow"),
                        Effect::LarsonScanner => info!("Effect: larson scanner"),
                        Effect::Off => info!("Effect: off"),
                    }
                }
                LedCommand::SetEnabled(new_enabled) => enabled = new_enabled,
                LedCommand::ShowWifiConnecting => {
                    overridden = true;
                    led_control.fill(super::YELLOW);
                }
                LedCommand::ShowWifiConnected => {
                    overridden = true;
                    led_control.fill(super::GREEN);
                }
                LedCommand::ShowWifiOff => {
                    overridden = true;
                    led_control.fill(super::RED);
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
                led_control.fill(super::BLACK);
            }
        }
    }
}
use led_task_mod::led_task;

#[embassy_executor::task]
async fn button_task(mut btn: Input<'static>) {
    // GPIO9 also doubles as the boot-mode strapping pin, which can read low briefly after
    // reset even without a real press; wait for it to actually be released before arming
    // the press detector below, instead of assuming it starts released.
    while btn.is_low() {
        btn.wait_for_rising_edge().await;
    }

    loop {
        btn.wait_for_falling_edge().await;
        match with_timeout(LONG_PRESS_DURATION, btn.wait_for_rising_edge()).await {
            Ok(()) => LED_CHANNEL.send(LedCommand::NextEffect).await,
            Err(_) => {
                // Still held past the threshold: toggle wifi/OTA on or off, then wait out
                // the release so it isn't also counted as a short press.
                info!("Long press detected, toggling wifi");
                WIFI_TOGGLE.signal(());
                btn.wait_for_rising_edge().await;
            }
        }
    }
}

#[embassy_executor::task]
async fn sensor_task(
    mut adxl_int2_pin: esp_hal::peripherals::GPIO4<'static>,
    mut accel: Accel,
    mut rtc: Rtc<'static>,
    mut ld_on: Output<'static>,
) {
    let mut adxl_int2 = Input::new(adxl_int2_pin.reborrow(), InputConfig::default());

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

// Owns the `WifiController` for its whole lifetime and toggles it on/off in response to
// `WIFI_TOGGLE`, rather than being spawned/dropped per toggle (the underlying esp-radio
// controller and embassy-net stack are only initialized once, in `main`).
#[embassy_executor::task]
async fn wifi_task(mut controller: WifiController<'static>, stack: embassy_net::Stack<'static>) {
    loop {
        LED_CHANNEL.send(LedCommand::ShowWifiConnecting).await;
        info!("Starting wifi");
        let client_config = ModeConfig::Client(
            ClientConfig::default()
                .with_ssid(wifi_secrets::SSID.into())
                .with_password(wifi_secrets::PASSWORD.into()),
        );
        controller.set_config(&client_config).unwrap();
        controller.start_async().await.unwrap();

        // Keep retrying to connect until either connected, or toggled off in the meantime.
        let connected = loop {
            info!("Connecting to wifi...");
            match select(controller.connect_async(), WIFI_TOGGLE.wait()).await {
                Either::First(Ok(())) => break true,
                Either::First(Err(e)) => {
                    info!("Failed to connect to wifi: {}, retrying", e);
                    match select(Timer::after(Duration::from_secs(5)), WIFI_TOGGLE.wait()).await {
                        Either::First(()) => continue,
                        Either::Second(()) => break false,
                    }
                }
                Either::Second(()) => break false,
            }
        };

        if connected {
            info!("Wifi connected");
            match select(wait_for_ip(stack), WIFI_TOGGLE.wait()).await {
                Either::First(()) => {
                    LED_CHANNEL.send(LedCommand::ShowWifiConnected).await;
                    // Stay connected until toggled off, or the AP drops us (then reconnect).
                    match select(
                        controller.wait_for_event(WifiEvent::StaDisconnected),
                        WIFI_TOGGLE.wait(),
                    )
                    .await
                    {
                        Either::First(()) => {
                            info!("Wifi disconnected, reconnecting");
                            continue;
                        }
                        Either::Second(()) => {}
                    }
                }
                Either::Second(()) => {}
            }
        }

        info!("Stopping wifi");
        let _ = controller.disconnect_async().await;
        let _ = controller.stop_async().await;
        LED_CHANNEL.send(LedCommand::ShowWifiOff).await;
        Timer::after(Duration::from_secs(1)).await;
        LED_CHANNEL.send(LedCommand::ResumeAnimation).await;

        WIFI_TOGGLE.wait().await;
    }
}

// Waits for link-up then an IP address, falling back to a static config if no DHCP lease
// shows up within `DHCP_TIMEOUT` (e.g. an AP with no DHCP server).
async fn wait_for_ip(stack: embassy_net::Stack<'static>) {
    stack.set_config_v4(ConfigV4::Dhcp(Default::default()));

    info!("Waiting for wifi link...");
    while !stack.is_link_up() {
        Timer::after(Duration::from_millis(500)).await;
    }

    info!("Waiting for IP address (DHCP)...");
    let dhcp_config = with_timeout(DHCP_TIMEOUT, async {
        loop {
            if let Some(config) = stack.config_v4() {
                return config;
            }
            Timer::after(Duration::from_millis(500)).await;
        }
    })
    .await;
    match dhcp_config {
        Ok(config) => info!("Got IP via DHCP: {}", config.address),
        Err(_) => {
            info!(
                "No DHCP lease after {}s, falling back to static IP",
                DHCP_TIMEOUT.as_secs()
            );
            stack.set_config_v4(ConfigV4::Static(StaticConfigV4 {
                address: Ipv4Cidr::new(STATIC_IP, 24),
                gateway: Some(STATIC_GATEWAY),
                dns_servers: Default::default(),
            }));
            info!("Using static IP: {}", STATIC_IP);
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
    let mut peripherals = esp_hal::init(config);

    // LD_ON pin: forced low immediately so LEDs stay off until fully initialized below.
    let mut ld_on = Output::new(
        peripherals.GPIO7,
        esp_hal::gpio::Level::Low,
        OutputConfig::default(),
    );
    ld_on.set_low();

    // Checked before any other init so a plain RTC-timer wakeup with nothing to do can
    // go straight back to sleep without powering up the LEDs, accelerometer, or Wi-Fi.
    let mut rtc = Rtc::new(peripherals.LPWR);
    match classify_wakeup() {
        WakeReason::TimerElapsed => {
            info!("RTC wakeup with nothing to do, going back to sleep");
            enter_deep_sleep(&mut rtc, &mut ld_on, &mut peripherals.GPIO4);
        }
        reason => info!("Wakeup reason: {}", reason),
    }

    let mut flash = FlashStorage::new(peripherals.FLASH);
    log_booted_partition(&mut flash);

    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    // BTN
    let btn = Input::new(
        peripherals.GPIO9,
        InputConfig::default().with_pull(Pull::Up),
    );

    // LED driver
    let rmt = Rmt::new(peripherals.RMT, Rate::from_mhz(80)).unwrap();
    let led_buffer = LED_BUF.init([PulseCode::default(); LED_BUFFER_SIZE]);
    let leds = SmartLedsAdapter::new(rmt.channel0, peripherals.GPIO8, led_buffer);
    let mut led_control = LedControl::new(leds);

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
    spawner
        .spawn(sensor_task(peripherals.GPIO4, accel, rtc, ld_on))
        .unwrap();

    // Wi-Fi/OTA stay uninitialized until the first long button press; see `WIFI_TOGGLE`.
    WIFI_TOGGLE.wait().await;

    esp_alloc::heap_allocator!(size: 64 * 1024);

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
    spawner.spawn(ota_task(net_stack, flash)).unwrap();
    spawner
        .spawn(wifi_task(wifi_controller, net_stack))
        .unwrap();

    loop {
        Timer::after(Duration::from_secs(3600)).await;
    }
}
