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
}
