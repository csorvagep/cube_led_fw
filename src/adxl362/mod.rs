//! Driver for the Analog Devices ADXL362 SPI accelerometer.

mod registers;

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

/// Measurement range for the acceleration filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum Range {
    G2,
    G4,
    G8,
}

/// Noise/power trade-off mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum NoiseMode {
    Normal,
    LowNoise,
    UltraLowNoise,
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
}
