//! Driver for the Analog Devices ADXL362 SPI accelerometer.

mod registers;

use embedded_hal::delay::DelayNs;
use embedded_hal::spi::SpiBus;
use registers as reg;

/// Errors returned by [`Adxl362`] operations.
#[derive(Debug, defmt::Format)]
pub enum Error<E> {
    Spi(E),
    /// A device ID register did not match the expected ADXL362 value.
    InvalidDeviceId,
}

impl<E> From<E> for Error<E> {
    fn from(err: E) -> Self {
        Error::Spi(err)
    }
}

/// A 3-axis acceleration reading (12-bit resolution, sign-extended to `i16`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
pub struct Acceleration {
    pub x: i16,
    pub y: i16,
    pub z: i16,
}

/// Flags decoded from the STATUS register.
#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
pub struct Status {
    pub data_ready: bool,
    pub awake: bool,
    pub act: bool,
    pub inact: bool,
}

/// Output data rate for the acceleration filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum OutputDataRate {
    Hz12_5,
    Hz25,
    Hz50,
    Hz100,
    Hz200,
    Hz400,
}

impl OutputDataRate {
    fn bits(self) -> u8 {
        match self {
            OutputDataRate::Hz12_5 => reg::FILTER_CTL_ODR_12_5_HZ,
            OutputDataRate::Hz25 => reg::FILTER_CTL_ODR_25_HZ,
            OutputDataRate::Hz50 => reg::FILTER_CTL_ODR_50_HZ,
            OutputDataRate::Hz100 => reg::FILTER_CTL_ODR_100_HZ,
            OutputDataRate::Hz200 => reg::FILTER_CTL_ODR_200_HZ,
            OutputDataRate::Hz400 => reg::FILTER_CTL_ODR_400_HZ,
        }
    }
}

/// Measurement range for the acceleration filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum Range {
    G2,
    G4,
    G8,
}

impl Range {
    fn bits(self) -> u8 {
        match self {
            Range::G2 => reg::FILTER_CTL_RANGE_2G,
            Range::G4 => reg::FILTER_CTL_RANGE_4G,
            Range::G8 => reg::FILTER_CTL_RANGE_8G,
        }
    }
}

/// Noise/power trade-off mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum NoiseMode {
    Normal,
    LowNoise,
    UltraLowNoise,
}

impl NoiseMode {
    fn bits(self) -> u8 {
        match self {
            NoiseMode::Normal => reg::POWER_CTL_NOISE_NORMAL,
            NoiseMode::LowNoise => reg::POWER_CTL_NOISE_LOW,
            NoiseMode::UltraLowNoise => reg::POWER_CTL_NOISE_ULTRALOW,
        }
    }
}

/// Configuration for the activity (motion start) detector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
pub struct ActivityConfig {
    /// 11-bit unsigned threshold the acceleration samples are compared against.
    pub threshold: u16,
    /// Activity timer value; the resulting time in seconds is `time / ODR`.
    pub time: u8,
    /// Compare against a captured reference instead of an absolute value.
    pub referenced: bool,
}

/// Configuration for the inactivity (motion end) detector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
pub struct InactivityConfig {
    /// 11-bit unsigned threshold the acceleration samples are compared against.
    pub threshold: u16,
    /// Inactivity timer value; the resulting time in seconds is `time / ODR`.
    pub time: u16,
    /// Compare against a captured reference instead of an absolute value.
    pub referenced: bool,
}

/// Selects which STATUS conditions are routed to an interrupt pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
pub struct InterruptConfig {
    /// Drive the pin active-low instead of active-high.
    pub active_low: bool,
    pub awake: bool,
    pub act: bool,
    pub inact: bool,
    pub data_ready: bool,
}

impl InterruptConfig {
    fn bits(self) -> u8 {
        let mut bits = 0;
        if self.data_ready {
            bits |= reg::INTMAP_DATA_READY;
        }
        if self.act {
            bits |= reg::INTMAP_ACT;
        }
        if self.inact {
            bits |= reg::INTMAP_INACT;
        }
        if self.awake {
            bits |= reg::INTMAP_AWAKE;
        }
        if self.active_low {
            bits |= reg::INTMAP_INT_LOW;
        }
        bits
    }
}

/// Driver for the ADXL362 accelerometer, generic over any `embedded-hal` SPI bus.
pub struct Adxl362<SPI> {
    spi: SPI,
}

// Maximum burst length used by any read in this driver (XDATA_L..ZDATA_H = 6 bytes).
const MAX_BURST_LEN: usize = 6;

impl<SPI> Adxl362<SPI>
where
    SPI: SpiBus<u8>,
{
    fn read_reg(&mut self, addr: u8) -> Result<u8, Error<SPI::Error>> {
        let mut buf = [reg::CMD_READ, addr, 0x00];
        self.spi.transfer_in_place(&mut buf)?;
        Ok(buf[2])
    }

    fn write_reg(&mut self, addr: u8, val: u8) -> Result<(), Error<SPI::Error>> {
        let mut buf = [reg::CMD_WRITE, addr, val];
        self.spi.transfer_in_place(&mut buf)?;
        Ok(())
    }

    fn read_regs(&mut self, addr: u8, out: &mut [u8]) -> Result<(), Error<SPI::Error>> {
        debug_assert!(out.len() <= MAX_BURST_LEN);
        let mut buf = [0u8; MAX_BURST_LEN + 2];
        buf[0] = reg::CMD_READ;
        buf[1] = addr;
        let n = out.len();
        self.spi.transfer_in_place(&mut buf[..n + 2])?;
        out.copy_from_slice(&buf[2..n + 2]);
        Ok(())
    }

    pub fn new(spi: SPI) -> Self {
        Self { spi }
    }

    /// Soft-resets the device and verifies its identification registers.
    pub fn init(&mut self, delay: &mut impl DelayNs) -> Result<(), Error<SPI::Error>> {
        // Force standby first: on a warm MCU reboot the ADXL362 may still be
        // mid-conversion from the previous session, which can desync the SPI
        // framing of the soft-reset command that follows.
        self.write_reg(reg::REG_POWER_CTL, reg::POWER_CTL_MEASURE_STANDBY)?;
        self.write_reg(reg::REG_SOFT_RESET, reg::SOFT_RESET_KEY)?;
        // Datasheet: allow 500us for the reset to complete before further access.
        delay.delay_us(500);

        if self.read_reg(reg::REG_DEVID_AD)? != reg::DEVID_AD_VAL {
            return Err(Error::InvalidDeviceId);
        }
        if self.read_reg(reg::REG_DEVID_MST)? != reg::DEVID_MST_VAL {
            return Err(Error::InvalidDeviceId);
        }
        if self.read_reg(reg::REG_PARTID)? != reg::PARTID_VAL {
            return Err(Error::InvalidDeviceId);
        }
        Ok(())
    }

    pub fn configure_filter(
        &mut self,
        odr: OutputDataRate,
        range: Range,
        noise: NoiseMode,
    ) -> Result<(), Error<SPI::Error>> {
        self.write_reg(reg::REG_FILTER_CTL, range.bits() | odr.bits())?;

        let power_ctl = self.read_reg(reg::REG_POWER_CTL)?;
        let power_ctl = (power_ctl & !reg::POWER_CTL_NOISE_MASK) | noise.bits();
        self.write_reg(reg::REG_POWER_CTL, power_ctl)
    }

    /// Switches the device between standby and continuous measurement mode.
    pub fn set_active(&mut self, active: bool) -> Result<(), Error<SPI::Error>> {
        let measure = if active {
            reg::POWER_CTL_MEASURE_ON
        } else {
            reg::POWER_CTL_MEASURE_STANDBY
        };
        let power_ctl = self.read_reg(reg::REG_POWER_CTL)?;
        let power_ctl = (power_ctl & !reg::POWER_CTL_MEASURE_MASK) | measure;
        self.write_reg(reg::REG_POWER_CTL, power_ctl)
    }

    pub fn read_acceleration(&mut self) -> Result<Acceleration, Error<SPI::Error>> {
        let mut buf = [0u8; 6];
        self.read_regs(reg::REG_XDATA_L, &mut buf)?;
        Ok(Acceleration {
            x: i16::from_le_bytes([buf[0], buf[1]]),
            y: i16::from_le_bytes([buf[2], buf[3]]),
            z: i16::from_le_bytes([buf[4], buf[5]]),
        })
    }

    pub fn read_status(&mut self) -> Result<Status, Error<SPI::Error>> {
        let status = self.read_reg(reg::REG_STATUS)?;
        Ok(Status {
            data_ready: status & reg::STATUS_DATA_RDY != 0,
            awake: status & reg::STATUS_AWAKE != 0,
            act: status & reg::STATUS_ACT != 0,
            inact: status & reg::STATUS_INACT != 0,
        })
    }

    pub fn configure_activity(&mut self, cfg: &ActivityConfig) -> Result<(), Error<SPI::Error>> {
        self.write_reg(reg::REG_THRESH_ACT_L, (cfg.threshold & 0xFF) as u8)?;
        self.write_reg(reg::REG_THRESH_ACT_H, ((cfg.threshold >> 8) & 0x07) as u8)?;
        self.write_reg(reg::REG_TIME_ACT, cfg.time)?;

        let act_inact_ctl = self.read_reg(reg::REG_ACT_INACT_CTL)?;
        let mut act_inact_ctl =
            (act_inact_ctl & !reg::ACT_INACT_CTL_ACT_REF) | reg::ACT_INACT_CTL_ACT_EN;
        if cfg.referenced {
            act_inact_ctl |= reg::ACT_INACT_CTL_ACT_REF;
        }
        self.write_reg(reg::REG_ACT_INACT_CTL, act_inact_ctl)
    }

    pub fn configure_inactivity(
        &mut self,
        cfg: &InactivityConfig,
    ) -> Result<(), Error<SPI::Error>> {
        self.write_reg(reg::REG_THRESH_INACT_L, (cfg.threshold & 0xFF) as u8)?;
        self.write_reg(reg::REG_THRESH_INACT_H, ((cfg.threshold >> 8) & 0x07) as u8)?;
        self.write_reg(reg::REG_TIME_INACT_L, (cfg.time & 0xFF) as u8)?;
        self.write_reg(reg::REG_TIME_INACT_H, ((cfg.time >> 8) & 0xFF) as u8)?;

        let act_inact_ctl = self.read_reg(reg::REG_ACT_INACT_CTL)?;
        let mut act_inact_ctl =
            (act_inact_ctl & !reg::ACT_INACT_CTL_INACT_REF) | reg::ACT_INACT_CTL_INACT_EN;
        if cfg.referenced {
            act_inact_ctl |= reg::ACT_INACT_CTL_INACT_REF;
        }
        self.write_reg(reg::REG_ACT_INACT_CTL, act_inact_ctl)
    }

    /// Enables linked mode: an activity event arms inactivity detection once,
    /// and the device returns to standby after the following inactivity event.
    /// Requires [`Self::configure_activity`] and [`Self::configure_inactivity`]
    /// to have been called first to enable the respective detectors.
    pub fn enable_linked_mode(&mut self) -> Result<(), Error<SPI::Error>> {
        let act_inact_ctl = self.read_reg(reg::REG_ACT_INACT_CTL)?;
        let act_inact_ctl = (act_inact_ctl & !reg::ACT_INACT_CTL_LINKLOOP_MASK)
            | reg::ACT_INACT_CTL_LINKLOOP_LINKED;
        self.write_reg(reg::REG_ACT_INACT_CTL, act_inact_ctl)
    }

    pub fn configure_interrupt1(&mut self, cfg: &InterruptConfig) -> Result<(), Error<SPI::Error>> {
        self.write_reg(reg::REG_INTMAP1, cfg.bits())
    }

    pub fn configure_interrupt2(&mut self, cfg: &InterruptConfig) -> Result<(), Error<SPI::Error>> {
        self.write_reg(reg::REG_INTMAP2, cfg.bits())
    }
}
