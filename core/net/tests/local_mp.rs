//! Two Wi-Fi controllers on melonDS's LocalMP medium (real FFI path): a
//! frame one transmits must land in the other's RX buffer.

use ds_net::{MpBackend, NetLink};
use ds_peripherals::spi::firmware::Firmware;
use ds_peripherals::wifi::WiFi;

const IO: u32 = 0x0480_0000;
/// ARM7 cycles in 5 ms.
const FIVE_MS: u32 = 33_513_982 / 200;

fn write_rf(wifi: &mut WiFi, id: u8, value: u8) {
    wifi.write(IO + 0x17E, (u16::from(id) << 8) | u16::from(value));
    wifi.write(IO + 0x17C, 5);
}

/// Powered, on channel 1 (the built-in firmware's RF type 3 tables), MAC
/// `00:09:BF:00:00:<last>`, RX into 0x4C00..0x5F00 with every filter open.
fn console(instance: usize, last: u8) -> WiFi {
    let mut fw = Firmware::new();
    fw.load(&ds_bios::firmware::FIRMWARE_DS);
    let mut wifi = WiFi::new();
    wifi.reset(&fw);
    wifi.set_link(Some(Box::new(
        NetLink::new(instance).with_pump(instance == 0),
    )));
    wifi.set_power_cnt(true);
    wifi.write(IO + 0x036, 0);

    write_rf(&mut wifi, 0x01, 0x4B);
    write_rf(&mut wifi, 0x02, 0x6C);
    assert_eq!(wifi.channel(), 1);

    for (i, half) in [0x0900_u16, 0x00BF, u16::from(last) << 8]
        .into_iter()
        .enumerate()
    {
        wifi.write(IO + 0x018 + 2 * i as u32, half);
    }
    wifi.write(IO + 0x050, 0x4C00);
    wifi.write(IO + 0x052, 0x5F00);
    wifi.write(IO + 0x056, 0x0C00 >> 1);
    wifi.write(IO + 0x0D0, 0xFFFF);
    wifi.write(IO + 0x0E0, 0);
    wifi.write(IO + 0x030, 0x8001);
    wifi
}

#[test]
fn frame_crosses_local_mp() {
    ds_net::init(MpBackend::Local);
    let mut host = console(0, 1);
    let mut guest = console(1, 2);

    // Broadcast data frame with a 4-byte body, TX header at Wi-Fi RAM 0.
    let length: u16 = 24 + 4 + 4;
    let mut frame = vec![0_u8; 12];
    frame[8] = 0x14;
    frame[0xA..0xC].copy_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(&[0x08, 0x00, 0, 0]);
    frame.extend_from_slice(&[0xFF; 6]);
    frame.extend_from_slice(&[0x00, 0x09, 0xBF, 0x00, 0x00, 0x01]);
    frame.extend_from_slice(&[0x00, 0x09, 0xBF, 0x00, 0x00, 0x01]);
    frame.extend_from_slice(&[0, 0, 0x12, 0x34, 0x56, 0x78, 0, 0, 0, 0]);
    for (i, pair) in frame.chunks(2).enumerate() {
        host.write(
            IO + 0x4000 + 2 * i as u32,
            u16::from_le_bytes([pair[0], pair[1]]),
        );
    }
    host.write(IO + 0x0A0, 0x8000); // W_TXBUF_LOC1
    host.write(IO + 0x0AE, 1); // W_TXREQ_SET

    host.new_frame();
    host.run(FIVE_MS);
    guest.new_frame();
    guest.run(FIVE_MS);

    assert_ne!(guest.read(IO + 0x010) & 1, 0, "RX IRQ");
    assert_eq!(guest.read(IO + 0x4C00 + 12), 0x0008, "frame control");
    assert_eq!(guest.read(IO + 0x4C00 + 12 + 24), 0x3412, "body");
    assert_eq!(guest.read(IO + 0x4C00 + 12 + 10), 0x0900, "sender MAC");

    // Powering Wi-Fi off leaves the medium (MP_End) without complaint.
    host.set_power_cnt(false);
    guest.set_power_cnt(false);
}
