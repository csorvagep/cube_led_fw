//! Deep-sleep policy: classifying why the CPU is running, and entering deep sleep.

use esp_hal::gpio::{Output, RtcPinWithResistors};
use esp_hal::rtc_cntl::sleep::{RtcioWakeupSource, TimerWakeupSource, WakeupLevel};
use esp_hal::rtc_cntl::{Rtc, SocResetReason, reset_reason, wakeup_cause};
use esp_hal::system::{Cpu, SleepSource};

/// How long the RTC timer waits before automatically waking the CPU again.
pub const WAKE_INTERVAL: core::time::Duration = core::time::Duration::from_secs(300);

/// Why the CPU is currently executing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum WakeReason {
    /// Power-on or any reset that wasn't a deep-sleep exit.
    ColdBoot,
    /// Woken by the accelerometer's INT2 pin (motion detected).
    Activity,
    /// Woken by the RTC timer with no external event: nothing to do, go back to sleep.
    TimerElapsed,
}

/// Classifies why the CPU is currently executing, based on the last reset/wakeup cause.
pub fn classify_wakeup() -> WakeReason {
    match (reset_reason(Cpu::ProCpu), wakeup_cause()) {
        (Some(SocResetReason::CoreDeepSleep), SleepSource::Timer) => WakeReason::TimerElapsed,
        (Some(SocResetReason::CoreDeepSleep), _) => WakeReason::Activity,
        _ => WakeReason::ColdBoot,
    }
}

/// Turns off LED power and puts the CPU into deep sleep, waking again after
/// [`WAKE_INTERVAL`] or immediately on a rising edge from `wake_pin` (the accelerometer's
/// INT2 line).
pub fn enter_deep_sleep(
    rtc: &mut Rtc<'_>,
    ld_on: &mut Output<'_>,
    wake_pin: &mut dyn RtcPinWithResistors,
) -> ! {
    ld_on.set_low();

    let timer = TimerWakeupSource::new(WAKE_INTERVAL);
    let mut pins: [(&mut dyn RtcPinWithResistors, WakeupLevel); 1] =
        [(wake_pin, WakeupLevel::High)];
    let rtcio = RtcioWakeupSource::new(&mut pins);

    rtc.sleep_deep(&[&timer, &rtcio]);
}
