#![cfg_attr(not(feature = "std"), no_std)]

// These modules use `defmt` for logging, which needs a global logger implementation
// (provided by `defmt-rtt` on the embedded target) to link. They aren't needed to test
// `layout3d` on the host, so they're excluded from host builds.
#[cfg(target_arch = "riscv32")]
pub mod adxl362;
pub mod layout3d;
#[cfg(target_arch = "riscv32")]
pub mod led_control;
pub mod vec3;
