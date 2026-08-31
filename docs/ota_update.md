# OTA Updates

## How it works

- Flash layout ([partitions.csv](../partitions.csv)): `factory`, `ota_0`, `ota_1` app slots
  (1MB each) plus an `otadata` partition that records which slot is currently selected.
- On boot, the 2nd-stage bootloader reads `otadata`: if both of its selector entries are
  blank, it boots `factory`; otherwise it boots whichever of `ota_0`/`ota_1` has the higher
  sequence number. This decision is independent of what's actually flashed in `factory` —
  once `otadata` points at an OTA slot, `factory` is only reachable again once `otadata` is
  reset.
- Plain USB flashing (`espflash flash`, i.e. `cargo run --release`) **always writes to
  `factory`** and never touches `otadata`. So after any OTA update, a USB reflash will
  silently have no effect until `otadata` is reset back to blank (see "Reset to factory"
  below).
- Wi-Fi and the OTA listener are off by default (to avoid Wi-Fi/RMT interrupt contention
  glitching the LEDs) and are toggled with a **long press (≥ 1s) of the button**:
  - 1st long press: turns Wi-Fi on. LEDs stop the current animation and turn solid
    **yellow** while connecting, then **green** once an IP address is obtained.
  - Another long press (from either yellow or green): disconnects/stops Wi-Fi, LEDs flash
    **red** for 1s, then resume whatever animation was running before.
  - Repeatable — toggles on/off each time.
- Once connected, the device listens for a TCP connection on port `3232`
  (`OTA_PORT` in [src/bin/main.rs](../src/bin/main.rs)). The wire protocol is a big-endian
  `u32` length prefix followed by that many raw bytes of an esp-idf app image (exactly what
  `espflash save-image` produces).
- On receiving a full image, the device writes it to whichever of `ota_0`/`ota_1` isn't
  currently active, updates `otadata` to select it, and reboots into the new firmware. If
  the transfer is interrupted, `otadata` is left untouched (the switch only happens after a
  fully successful transfer), so a retry is always safe.

## Building and flashing over USB

Builds and flashes to `factory` over a USB cable, and opens a serial monitor:

```
cargo run --bin cubeled --release
```

## Pushing an update over OTA

1. Build the new firmware:
   ```
   cargo build --release
   ```
2. Long-press the device's button until the LEDs turn green (connected, with an IP).
3. Push the build to the device (`ota_flash` is a host-only tool, see
   [src/bin/ota_flash.rs](../src/bin/ota_flash.rs)):
   ```
   cargo run --bin ota_flash --release --target x86_64-pc-windows-msvc --features std -- <device-ip>
   ```
   `<device-ip>` is whatever the device logged after connecting (defmt log line
   `Got IP via DHCP: ...`, visible over USB). The device reboots into the new firmware
   automatically once the transfer completes.

## Reset to factory image

OTA updates only ever target `ota_0`/`ota_1` — they never touch or select `factory` — so
`factory` always holds whatever was last flashed over USB, untouched by any OTA push. To
force the device back to booting `factory`, reset the `otadata` partition from the host
(works over USB regardless of what firmware is currently running, or even if it's
unresponsive):

```
espflash erase-parts otadata --partition-table partitions.csv --chip esp32c3
```

Then reflash good firmware with `cargo run --release` (which targets `factory`), and the
next boot will see `otadata` blank and boot `factory`.
