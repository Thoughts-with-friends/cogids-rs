use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use super::*;

type Queue = Arc<Mutex<VecDeque<(Vec<u8>, u64)>>>;

/// One end of an in-memory two-console medium.
#[derive(Debug, Default)]
struct PairLink {
    out: Queue,
    inp: Queue,
    begun: bool,
}

impl PairLink {
    fn pair() -> (Self, Self) {
        let (a, b) = (Queue::default(), Queue::default());
        (
            Self {
                out: Arc::clone(&a),
                inp: Arc::clone(&b),
                begun: false,
            },
            Self {
                out: b,
                inp: a,
                begun: false,
            },
        )
    }

    fn push(&self, frame: &[u8], timestamp: u64) {
        self.out
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back((frame.to_vec(), timestamp));
    }

    fn pop(&self, buf: &mut [u8]) -> MpRecv {
        let next = self
            .inp
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front();
        next.map_or(MpRecv::Empty, |(frame, timestamp)| {
            buf[..frame.len()].copy_from_slice(&frame);
            MpRecv::Frame {
                len: frame.len(),
                timestamp,
            }
        })
    }
}

impl MpLink for PairLink {
    fn begin(&mut self) {
        self.begun = true;
    }
    fn end(&mut self) {
        self.begun = false;
    }
    fn send_packet(&mut self, frame: &[u8], timestamp: u64) {
        self.push(frame, timestamp);
    }
    fn send_cmd(&mut self, frame: &[u8], timestamp: u64) {
        self.push(frame, timestamp);
    }
    fn send_reply(&mut self, frame: &[u8], timestamp: u64, _aid: u16) {
        if !frame.is_empty() {
            self.push(frame, timestamp);
        }
    }
    fn send_ack(&mut self, frame: &[u8], timestamp: u64) {
        self.push(frame, timestamp);
    }
    fn recv_packet(&mut self, buf: &mut [u8]) -> MpRecv {
        self.pop(buf)
    }
    fn recv_host_packet(&mut self, buf: &mut [u8]) -> MpRecv {
        self.pop(buf)
    }
    fn recv_replies(&mut self, _buf: &mut [u8], _timestamp: u64, _aidmask: u16) -> u16 {
        0
    }
}

/// RF type 3 channel tables of melonDS's generated firmware (the same bytes
/// as `ds_bios::firmware`).
const RF_INDEX: [u8; 2] = [0x01, 0x02];
const RF_DATA1: [u8; 14] = [
    0x4B, 0x4B, 0x4B, 0x4B, 0x4C, 0x4C, 0x4C, 0x4C, 0x4C, 0x4C, 0x4C, 0x4D, 0x4D, 0x4D,
];
const RF_DATA2: [u8; 14] = [
    0x6C, 0x71, 0x76, 0x5B, 0x40, 0x45, 0x4A, 0x2F, 0x34, 0x39, 0x3E, 0x03, 0x08, 0x14,
];

fn firmware() -> Firmware {
    let mut fw = Firmware::new();
    let raw = &mut fw.raw_firmware;
    raw[0x1D] = 0xFF;
    raw[0x40] = 3;
    raw[0x116] = RF_INDEX[0];
    raw[0x117..0x117 + 14].copy_from_slice(&RF_DATA1);
    raw[0x125] = RF_INDEX[1];
    raw[0x126..0x126 + 14].copy_from_slice(&RF_DATA2);
    fw
}

const IO: u32 = 0x0480_0000;

/// Programs a type 3 RF register like the wireless library does.
fn write_rf(wifi: &mut WiFi, id: u8, value: u8) {
    wifi.write(
        IO + W_RF_DATA1 as u32,
        (u16::from(id) << 8) | u16::from(value),
    );
    wifi.write(IO + W_RF_DATA2 as u32, 5);
}

fn tune(wifi: &mut WiFi, channel: usize) {
    write_rf(wifi, RF_INDEX[0], RF_DATA1[channel - 1]);
    write_rf(wifi, RF_INDEX[1], RF_DATA2[channel - 1]);
}

/// A powered controller with MAC `00:09:BF:00:00:<last>`, tuned to channel
/// 1, RX buffer at 0x4C00..0x5F00 accepting everything.
fn console(link: PairLink, last: u8) -> WiFi {
    let mut wifi = WiFi::new();
    wifi.reset(&firmware());
    wifi.set_link(Some(Box::new(link)));
    wifi.set_power_cnt(true);
    wifi.write(IO + W_POWER_US as u32, 0);
    tune(&mut wifi, 1);
    for (i, half) in [0x0900_u16, 0x00BF, u16::from(last) << 8]
        .into_iter()
        .enumerate()
    {
        wifi.write(IO + (W_MACADDR_0 + i * 2) as u32, half);
    }
    wifi.write(IO + W_RXBUF_BEGIN as u32, 0x4C00);
    wifi.write(IO + W_RXBUF_END as u32, 0x5F00);
    wifi.write(IO + W_RXBUF_WR_ADDR as u32, 0x0C00 >> 1);
    wifi.write(IO + W_RXFILTER as u32, 0xFFFF);
    wifi.write(IO + W_RXFILTER2 as u32, 0);
    wifi.write(IO + W_RXCNT as u32, 0x8001);
    wifi
}

/// ARM7 cycles in `ms` milliseconds.
const fn ms(ms: u64) -> u32 {
    (ARM7_HZ * ms / 1000) as u32
}

#[test]
fn channel_follows_rf_registers() {
    let mut wifi = WiFi::new();
    wifi.reset(&firmware());
    assert_eq!(wifi.channel(), 0);
    for channel in [1, 7, 13] {
        tune(&mut wifi, channel);
        assert_eq!(usize::from(wifi.channel()), channel);
    }
    write_rf(&mut wifi, RF_INDEX[1], 0xEE);
    assert_eq!(wifi.channel(), 0);
}

#[test]
fn link_follows_power() {
    let (a, _b) = PairLink::pair();
    let mut wifi = console(a, 1);
    assert!(wifi.link_begun);
    wifi.set_power_cnt(false);
    assert!(!wifi.link_begun);
}

#[test]
fn broadcast_frame_reaches_the_other_console() {
    let (a, b) = PairLink::pair();
    let mut host = console(a, 1);
    let mut guest = console(b, 2);

    // TX header + 802.11 data frame to the broadcast address, at 0x000.
    let body = [0xDE_u8, 0xAD, 0xBE, 0xEF];
    let length = (24 + body.len() + 4) as u16;
    let mut frame = vec![0_u8; 12];
    frame[8] = 0x14; // 2 Mbit/s
    frame[0xA..0xC].copy_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(&0x0008_u16.to_le_bytes()); // data
    frame.extend_from_slice(&[0, 0]);
    frame.extend_from_slice(&[0xFF; 6]);
    frame.extend_from_slice(&[0x00, 0x09, 0xBF, 0x00, 0x00, 0x01]);
    frame.extend_from_slice(&[0x00, 0x09, 0xBF, 0x00, 0x00, 0x01]);
    frame.extend_from_slice(&[0, 0]);
    frame.extend_from_slice(&body);
    frame.extend_from_slice(&[0; 4]);
    for (i, pair) in frame.chunks(2).enumerate() {
        let half = u16::from_le_bytes([pair[0], *pair.get(1).unwrap_or(&0)]);
        host.write(IO + 0x4000 + (i * 2) as u32, half);
    }
    host.write(IO + W_TXBUF_LOC1 as u32, 0x8000);
    host.write(IO + W_TXREQ_SET as u32, 1);

    host.run(ms(5));
    assert_eq!(
        host.read(IO + W_TXBUF_LOC1 as u32) & 0x8000,
        0,
        "slot freed"
    );
    assert_ne!(host.port(W_IF) & (1 << 1), 0, "TX IRQ");

    guest.run(ms(5));
    assert_ne!(guest.port(W_IF) & 1, 0, "RX IRQ");
    // RX header (flags, rate, length) then the frame, at the write cursor.
    let at = 0x4C00;
    assert_eq!(guest.read(IO + at) & 0x000F, 0x0008, "data frame flags");
    assert_eq!(guest.read(IO + at + 8), length);
    assert_eq!(guest.read(IO + at + 12), 0x0008, "frame control");
    assert_eq!(guest.read(IO + at + 12 + 24), 0xADDE);
    let wrcsr = u32::from(guest.read(IO + W_RXBUF_WRCSR as u32)) << 1;
    assert_eq!(wrcsr, 0x0C00 + ((12 + u32::from(length) + 3) & !3));
}

#[test]
fn untuned_console_sends_nothing() {
    let (a, b) = PairLink::pair();
    let mut host = console(a, 1);
    write_rf(&mut host, RF_INDEX[0], 0); // off-channel
    host.write(IO + 0x4000 + 0xA, 28);
    host.write(IO + W_TXBUF_LOC1 as u32, 0x8000);
    host.write(IO + W_TXREQ_SET as u32, 1);
    host.run(ms(5));
    assert!(
        b.inp
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    );
}
