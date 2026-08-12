//! ADXL362 register addresses and bitfield constants.
//! Verified against the Analog Devices no-OS and Zephyr ADXL362 drivers.

// SPI command bytes (sent as the first byte of every transaction).
pub const CMD_WRITE: u8 = 0x0A;
pub const CMD_READ: u8 = 0x0B;
pub const CMD_READ_FIFO: u8 = 0x0D;

// Register addresses.
pub const REG_DEVID_AD: u8 = 0x00;
pub const REG_DEVID_MST: u8 = 0x01;
pub const REG_PARTID: u8 = 0x02;
pub const REG_REVID: u8 = 0x03;
pub const REG_XDATA: u8 = 0x08;
pub const REG_YDATA: u8 = 0x09;
pub const REG_ZDATA: u8 = 0x0A;
pub const REG_STATUS: u8 = 0x0B;
pub const REG_FIFO_ENTRIES_L: u8 = 0x0C;
pub const REG_FIFO_ENTRIES_H: u8 = 0x0D;
pub const REG_XDATA_L: u8 = 0x0E;
pub const REG_XDATA_H: u8 = 0x0F;
pub const REG_YDATA_L: u8 = 0x10;
pub const REG_YDATA_H: u8 = 0x11;
pub const REG_ZDATA_L: u8 = 0x12;
pub const REG_ZDATA_H: u8 = 0x13;
pub const REG_TEMP_L: u8 = 0x14;
pub const REG_TEMP_H: u8 = 0x15;
pub const REG_SOFT_RESET: u8 = 0x1F;
pub const REG_THRESH_ACT_L: u8 = 0x20;
pub const REG_THRESH_ACT_H: u8 = 0x21;
pub const REG_TIME_ACT: u8 = 0x22;
pub const REG_THRESH_INACT_L: u8 = 0x23;
pub const REG_THRESH_INACT_H: u8 = 0x24;
pub const REG_TIME_INACT_L: u8 = 0x25;
pub const REG_TIME_INACT_H: u8 = 0x26;
pub const REG_ACT_INACT_CTL: u8 = 0x27;
pub const REG_FIFO_CTL: u8 = 0x28;
pub const REG_FIFO_SAMPLES: u8 = 0x29;
pub const REG_INTMAP1: u8 = 0x2A;
pub const REG_INTMAP2: u8 = 0x2B;
pub const REG_FILTER_CTL: u8 = 0x2C;
pub const REG_POWER_CTL: u8 = 0x2D;
pub const REG_SELF_TEST: u8 = 0x2E;

// Expected identification values.
pub const DEVID_AD_VAL: u8 = 0xAD;
pub const DEVID_MST_VAL: u8 = 0x1D;
pub const PARTID_VAL: u8 = 0xF2;

// REG_SOFT_RESET: writing this key triggers a reset.
pub const SOFT_RESET_KEY: u8 = 0x52;

// REG_STATUS bitfield.
pub const STATUS_DATA_RDY: u8 = 1 << 0;
pub const STATUS_FIFO_RDY: u8 = 1 << 1;
pub const STATUS_FIFO_WATERMARK: u8 = 1 << 2;
pub const STATUS_FIFO_OVERRUN: u8 = 1 << 3;
pub const STATUS_ACT: u8 = 1 << 4;
pub const STATUS_INACT: u8 = 1 << 5;
pub const STATUS_AWAKE: u8 = 1 << 6;
pub const STATUS_ERR_USER_REGS: u8 = 1 << 7;

// REG_ACT_INACT_CTL bitfield.
pub const ACT_INACT_CTL_ACT_EN: u8 = 1 << 0;
pub const ACT_INACT_CTL_ACT_REF: u8 = 1 << 1;
pub const ACT_INACT_CTL_INACT_EN: u8 = 1 << 2;
pub const ACT_INACT_CTL_INACT_REF: u8 = 1 << 3;
// LINKLOOP occupies bits [5:4].
pub const ACT_INACT_CTL_LINKLOOP_MASK: u8 = 0b11 << 4;
pub const ACT_INACT_CTL_LINKLOOP_DEFAULT: u8 = 0b00 << 4;
pub const ACT_INACT_CTL_LINKLOOP_LINKED: u8 = 0b01 << 4;
pub const ACT_INACT_CTL_LINKLOOP_LOOP: u8 = 0b11 << 4;

// REG_INTMAP1 / REG_INTMAP2 share the same bitfield layout.
pub const INTMAP_DATA_READY: u8 = 1 << 0;
pub const INTMAP_FIFO_READY: u8 = 1 << 1;
pub const INTMAP_FIFO_WATERMARK: u8 = 1 << 2;
pub const INTMAP_FIFO_OVERRUN: u8 = 1 << 3;
pub const INTMAP_ACT: u8 = 1 << 4;
pub const INTMAP_INACT: u8 = 1 << 5;
pub const INTMAP_AWAKE: u8 = 1 << 6;
pub const INTMAP_INT_LOW: u8 = 1 << 7;

// REG_FILTER_CTL bitfield.
// ODR occupies bits [2:0].
pub const FILTER_CTL_ODR_12_5_HZ: u8 = 0;
pub const FILTER_CTL_ODR_25_HZ: u8 = 1;
pub const FILTER_CTL_ODR_50_HZ: u8 = 2;
pub const FILTER_CTL_ODR_100_HZ: u8 = 3;
pub const FILTER_CTL_ODR_200_HZ: u8 = 4;
pub const FILTER_CTL_ODR_400_HZ: u8 = 5;
pub const FILTER_CTL_EXT_SAMPLE: u8 = 1 << 3;
pub const FILTER_CTL_HALF_BW: u8 = 1 << 4;
// RANGE occupies bits [7:6].
pub const FILTER_CTL_RANGE_2G: u8 = 0 << 6;
pub const FILTER_CTL_RANGE_4G: u8 = 1 << 6;
pub const FILTER_CTL_RANGE_8G: u8 = 2 << 6;

// REG_POWER_CTL bitfield.
// MEASURE occupies bits [1:0].
pub const POWER_CTL_MEASURE_MASK: u8 = 0b11;
pub const POWER_CTL_MEASURE_STANDBY: u8 = 0;
pub const POWER_CTL_MEASURE_ON: u8 = 0b10;
pub const POWER_CTL_AUTOSLEEP: u8 = 1 << 2;
pub const POWER_CTL_WAKEUP: u8 = 1 << 3;
// LOW_NOISE occupies bits [5:4].
pub const POWER_CTL_NOISE_MASK: u8 = 0b11 << 4;
pub const POWER_CTL_NOISE_NORMAL: u8 = 0b00 << 4;
pub const POWER_CTL_NOISE_LOW: u8 = 0b01 << 4;
pub const POWER_CTL_NOISE_ULTRALOW: u8 = 0b10 << 4;
pub const POWER_CTL_EXT_CLK: u8 = 1 << 6;
