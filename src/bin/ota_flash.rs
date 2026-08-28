// Host-only tool: builds a flashable image via `espflash save-image` and pushes it to the
// device's OTA TCP listener (see `ota_task` in src/bin/main.rs) instead of over USB.
//
// Usage: cargo run --bin ota_flash --release --target <host-triple> --features std -- <device-ip[:port]> [path-to-elf]

use std::io::Write;
use std::net::TcpStream;
use std::process::Command;

const DEFAULT_ELF: &str = "target/riscv32imc-unknown-none-elf/release/cubeled";
const DEFAULT_PORT: u16 = 3232;
const PARTITION_TABLE: &str = "partitions.csv";

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(host) = args.next() else {
        eprintln!("usage: ota_flash <device-ip[:port]> [path-to-elf]");
        std::process::exit(1);
    };
    let elf = args.next().unwrap_or_else(|| DEFAULT_ELF.to_string());
    let addr = if host.contains(':') {
        host
    } else {
        format!("{host}:{DEFAULT_PORT}")
    };

    let image_path = std::env::temp_dir().join("cubeled_ota_image.bin");

    println!("Building OTA image from {elf}...");
    let status = Command::new("espflash")
        .args([
            "save-image",
            "--chip",
            "esp32c3",
            "--partition-table",
            PARTITION_TABLE,
            &elf,
        ])
        .arg(&image_path)
        .status()
        .expect("failed to run espflash - is it installed and on PATH?");
    if !status.success() {
        eprintln!("espflash save-image failed");
        std::process::exit(1);
    }

    let image = std::fs::read(&image_path).expect("failed to read generated OTA image");

    println!("Connecting to {addr}...");
    let mut stream = TcpStream::connect(&addr).expect("failed to connect to device");

    println!("Sending {} bytes...", image.len());
    stream
        .write_all(&(image.len() as u32).to_be_bytes())
        .expect("failed to send image length");
    stream.write_all(&image).expect("failed to send image");
    stream.flush().expect("failed to flush");

    println!("OTA image sent; device should reboot into the new firmware.");
}
