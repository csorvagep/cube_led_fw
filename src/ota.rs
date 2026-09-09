use defmt::info;
use embassy_net::tcp::TcpSocket;
use embassy_net::{IpListenEndpoint, Stack};
use embassy_time::{Duration, Timer};
use embedded_io_async::Read;
use embedded_storage::Storage;
use esp_bootloader_esp_idf::ota::OtaImageState;
use esp_bootloader_esp_idf::ota_updater::OtaUpdater;
use esp_bootloader_esp_idf::partitions::{
    AppPartitionSubType, PARTITION_TABLE_MAX_LEN, PartitionType, read_partition_table,
};
use esp_hal::system::software_reset;
use esp_storage::FlashStorage;

use crate::led_control::{LED_CHANNEL, LedCommand};

const OTA_PORT: u16 = 3232;

#[embassy_executor::task]
pub async fn ota_task(stack: Stack<'static>, mut flash: FlashStorage<'static>) {
    mark_current_slot_valid(&mut flash);

    let mut rx_buffer = [0u8; 4096];
    let mut tx_buffer = [0u8; 256];
    let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_timeout(Some(Duration::from_secs(30)));

    loop {
        info!("Waiting for OTA connection on port {}", OTA_PORT);
        if socket
            .accept(IpListenEndpoint {
                addr: None,
                port: OTA_PORT,
            })
            .await
            .is_err()
        {
            continue;
        }
        info!("OTA client connected");
        LED_CHANNEL.send(LedCommand::ShowOtaInProgress).await;

        match receive_ota_image(&mut socket, &mut flash).await {
            Ok(()) => {
                info!("OTA image applied, rebooting into new firmware");
                LED_CHANNEL.send(LedCommand::ShowOtaSuccess).await;
                let _ = socket.flush().await;
                Timer::after(Duration::from_millis(200)).await;
                software_reset();
            }
            Err(e) => {
                info!("OTA update failed: {}", e);
                LED_CHANNEL.send(LedCommand::ResumeAnimation).await;
            }
        }
        socket.close();
    }
}


// Only relevant with a rollback-capable bootloader (the prebuilt one espflash ships isn't),
// but marking the slot valid here is cheap insurance either way.
fn mark_current_slot_valid(flash: &mut FlashStorage<'static>) {
    let mut pt_buf = [0u8; PARTITION_TABLE_MAX_LEN];
    let Ok(mut ota) = OtaUpdater::new(flash, &mut pt_buf) else {
        info!("No OTA partitions found; OTA updates are unavailable");
        return;
    };
    if matches!(
        ota.current_ota_state(),
        Ok(OtaImageState::New) | Ok(OtaImageState::PendingVerify)
    ) {
        let _ = ota.set_current_ota_state(OtaImageState::Valid);
    }
}

// Wire protocol: a big-endian u32 image length prefix, followed by that many raw
// esp-idf app image bytes (i.e. exactly what `espflash save-image` produces).
async fn receive_ota_image(
    socket: &mut TcpSocket<'_>,
    flash: &mut FlashStorage<'static>,
) -> Result<(), &'static str> {
    let mut len_buf = [0u8; 4];
    socket
        .read_exact(&mut len_buf)
        .await
        .map_err(|_| "failed to read image length")?;
    let total_len = u32::from_be_bytes(len_buf) as usize;
    info!("Receiving OTA image: {} bytes", total_len);

    let mut pt_buf = [0u8; PARTITION_TABLE_MAX_LEN];
    let mut ota =
        OtaUpdater::new(flash, &mut pt_buf).map_err(|_| "failed to read partition table")?;
    let (mut next_partition, part_type) =
        ota.next_partition().map_err(|_| "no free OTA partition")?;
    info!("Writing image to {}", part_type);

    let mut chunk = [0u8; 4096];
    let mut received = 0usize;
    while received < total_len {
        let want = (total_len - received).min(chunk.len());
        socket
            .read_exact(&mut chunk[..want])
            .await
            .map_err(|_| "connection error while receiving image")?;
        next_partition
            .write(received as u32, &chunk[..want])
            .map_err(|_| "flash write failed")?;
        received += want;
    }

    ota.activate_next_partition()
        .map_err(|_| "failed to activate new partition")?;
    ota.set_current_ota_state(OtaImageState::New)
        .map_err(|_| "failed to set OTA state")?;

    Ok(())
}

// Logs which A/B slot is actually running (independent of what the OTA-data partition
// *thinks* is selected), since plain USB flashes always target `factory` and silently have
// no effect once OTA has moved the boot slot to `ota_0`/`ota_1` -- this makes that visible.
pub fn log_booted_partition(flash: &mut FlashStorage<'static>) {
    let mut pt_buf = [0u8; PARTITION_TABLE_MAX_LEN];
    let booted = read_partition_table(flash, &mut pt_buf).and_then(|pt| pt.booted_partition());
    match booted {
        Ok(Some(part)) => {
            info!("Booted from partition: {}", part.partition_type());
            if part.partition_type() != PartitionType::App(AppPartitionSubType::Factory) {
                info!(
                    "Note: USB flashes always write to 'factory' and won't take effect until \
                     OTA selection is reset back to it"
                );
            }
        }
        Ok(None) => info!("Could not determine the booted partition"),
        Err(e) => info!("Failed to determine the booted partition: {}", e),
    }
}
