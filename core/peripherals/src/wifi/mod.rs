//! Wi-Fi controller (ARM7 only, 0x04800000..0x0480FFFF).
//!
//! ```text
//!  ARM7 wireless library (WM / wl, running in ARM7 RAM)
//!    |  16-bit reads/writes
//!    v
//!  0x04800000-0x04800FFF  I/O ports (mirrored at 0x04808000)   -> `io`
//!  0x04804000-0x04805FFF  8 KB Wi-Fi RAM (TX/RX buffers)       -> `ram`
//!    |
//!    +-- W_BB_CNT / W_BB_WRITE / W_BB_READ  -> baseband chip (`bb_regs`)
//!    +-- W_RF_DATA1 / W_RF_DATA2           -> RF chip (`rf_regs`, channel)
//!    +-- W_POWER_* / W_MODE_RST            -> transceiver power state
//!    +-- W_US_COUNT / W_US_COMPARE         -> microsecond timer, IRQ 14/15
//!    +-- W_TXBUF_LOC1..3 / CMD / BEACON    -> TX slots          (`tx`)
//!    +-- W_RXBUF_*                         -> RX ring buffer    (`rx`)
//!    `-- W_IF / W_IE                       -> ARM7 IRQ 24 (Wi-Fi)
//!
//!  frames <-> dyn MpLink (`link`): the other consoles (ds_net / melonDS MP)
//! ```
//!
//! A port of melonDS `Wifi.cpp`. Without a link ([`WiFi::set_link`]) frames
//! go nowhere, which looks to software like a console with nobody in range;
//! that is still enough for `WM_Enable`/`WM_PowerOn` to succeed (with the old
//! all-zero stub the ARM7 library reported a hardware failure and Pokemon
//! HeartGold disabled the touch screen on its title menu).
//!
//! Not ported: the emulated access point (`WifiAP`, Nintendo WFC).
//!
//! Timing: [`WiFi::run`] is fed ARM7 bus cycles and advances the
//! microsecond timer in `US_STEP` µs steps while the controller is powered
//! (POWCNT2 bit 1 set and W_POWER_US bit 0 clear). TX and RX progress on the
//! same steps, as on melonDS's `USTimer` event.

mod link;
mod rx;
mod tx;

pub use link::{MpLink, MpRecv};

use crate::spi::firmware::Firmware;

/// ARM7 bus cycles per second.
const ARM7_HZ: u64 = 33_513_982;

/// Microseconds advanced per timer step (melonDS `kTimerInterval`).
const US_STEP: u32 = 8;

/// Mask that is zero for the first step of each block (melonDS `kTimeCheckMask`).
const STEP_MASK: u64 = !(US_STEP as u64 - 1);

// I/O port offsets (GBATEK "DS Wifi I/O Map").
const W_ID: usize = 0x000;
const W_MODE_RST: usize = 0x004;
const W_MODE_WEP: usize = 0x006;
const W_TXSTATCNT: usize = 0x008;
const W_IF: usize = 0x010;
const W_IE: usize = 0x012;
const W_MACADDR_0: usize = 0x018;
const W_BSSID_0: usize = 0x020;
const W_AID_LOW: usize = 0x028;
const W_AID_FULL: usize = 0x02A;
const W_TX_RETRYLIMIT: usize = 0x02C;
const W_RXCNT: usize = 0x030;
const W_WEPCNT: usize = 0x032;
const W_TRX_POWER: usize = 0x034;
const W_POWER_US: usize = 0x036;
const W_POWER_TX: usize = 0x038;
const W_POWERSTATE: usize = 0x03C;
const W_POWERFORCE: usize = 0x040;
const W_RANDOM: usize = 0x044;
const W_POWERDOWNCTRL: usize = 0x048;
const W_RXBUF_BEGIN: usize = 0x050;
const W_RXBUF_END: usize = 0x052;
const W_RXBUF_WRCSR: usize = 0x054;
const W_RXBUF_WR_ADDR: usize = 0x056;
const W_RXBUF_RD_ADDR: usize = 0x058;
const W_RXBUF_READCSR: usize = 0x05A;
const W_RXBUF_COUNT: usize = 0x05C;
const W_RXBUF_RD_DATA: usize = 0x060;
const W_RXBUF_GAP: usize = 0x062;
const W_RXBUF_GAPDISP: usize = 0x064;
const W_TXBUF_WR_ADDR: usize = 0x068;
const W_TXBUF_COUNT: usize = 0x06C;
const W_TXBUF_WR_DATA: usize = 0x070;
const W_TXBUF_GAP: usize = 0x074;
const W_TXBUF_GAPDISP: usize = 0x076;
const W_TXBUF_BEACON: usize = 0x080;
const W_TXBUF_TIM: usize = 0x084;
const W_LISTENCOUNT: usize = 0x088;
const W_BEACONINT: usize = 0x08C;
const W_LISTENINT: usize = 0x08E;
const W_TXBUF_CMD: usize = 0x090;
const W_TXBUF_REPLY1: usize = 0x094;
const W_TXBUF_REPLY2: usize = 0x098;
const W_TXBUF_LOC1: usize = 0x0A0;
const W_TXBUF_LOC2: usize = 0x0A4;
const W_TXBUF_LOC3: usize = 0x0A8;
const W_TXREQ_RESET: usize = 0x0AC;
const W_TXREQ_SET: usize = 0x0AE;
const W_TXREQ_READ: usize = 0x0B0;
const W_TXBUF_RESET: usize = 0x0B4;
const W_TXBUSY: usize = 0x0B6;
const W_TXSTAT: usize = 0x0B8;
const W_PREAMBLE: usize = 0x0BC;
const W_CMD_TOTALTIME: usize = 0x0C0;
const W_CMD_REPLYTIME: usize = 0x0C4;
const W_RXFILTER: usize = 0x0D0;
const W_RXLEN_CROP: usize = 0x0DA;
const W_RXFILTER2: usize = 0x0E0;
const W_US_COUNTCNT: usize = 0x0E8;
const W_US_COMPARECNT: usize = 0x0EA;
const W_CMD_COUNTCNT: usize = 0x0EE;
const W_US_COMPARE0: usize = 0x0F0;
const W_US_COMPARE1: usize = 0x0F2;
const W_US_COMPARE2: usize = 0x0F4;
const W_US_COMPARE3: usize = 0x0F6;
const W_US_COUNT0: usize = 0x0F8;
const W_US_COUNT1: usize = 0x0FA;
const W_US_COUNT2: usize = 0x0FC;
const W_US_COUNT3: usize = 0x0FE;
const W_CONTENTFREE: usize = 0x10C;
const W_PRE_BEACON: usize = 0x110;
const W_CMD_COUNT: usize = 0x118;
const W_BEACON_COUNT1: usize = 0x11C;
const W_BEACON_COUNT2: usize = 0x134;
const W_BB_CNT: usize = 0x158;
const W_BB_WRITE: usize = 0x15A;
const W_BB_READ: usize = 0x15C;
const W_BB_BUSY: usize = 0x15E;
const W_RF_DATA2: usize = 0x17C;
const W_RF_DATA1: usize = 0x17E;
const W_RF_BUSY: usize = 0x180;
const W_RF_CNT: usize = 0x184;
const W_TX_HDR_CNT: usize = 0x194;
const W_RF_PINS: usize = 0x19C;
const W_CMD_STAT0: usize = 0x1D0;
const W_CMD_STAT7: usize = 0x1DE;
const W_TX_SEQNO: usize = 0x210;
const W_RF_STATUS: usize = 0x214;
const W_IF_SET: usize = 0x21C;
const W_RXTX_ADDR: usize = 0x268;

/// One transmit slot (melonDS `TXSlot`): LOC1, CMD, LOC2, LOC3, beacon,
/// MP reply.
#[derive(Debug, Default, Clone, Copy)]
struct TxSlot {
    valid: bool,
    /// Wi-Fi RAM offset of the 12-byte TX header.
    addr: usize,
    /// 802.11 frame length (header + body + FCS).
    length: u16,
    /// 1 = 1 Mbit/s, 2 = 2 Mbit/s.
    rate: u8,
    cur_phase: u8,
    cur_phase_time: i32,
    halfword_time_mask: u32,
}

/// Wi-Fi controller state (see the module docs).
#[derive(Debug)]
pub struct WiFi {
    /// I/O ports 0x000..0xFFF as halfwords.
    io: [u16; 0x800],
    /// 8 KB Wi-Fi RAM (0x04804000).
    ram: Vec<u8>,

    /// Baseband chip registers and which of them are read-only.
    bb_regs: [u8; 0x100],
    bb_read_only: [bool; 0x100],
    /// RF chip registers and the chip type from the firmware (2 or 3).
    rf_regs: [u32; 0x40],
    rf_version: u8,
    /// The two RF registers that select the channel, and their value per
    /// channel 1..=14 (from the firmware's Wi-Fi calibration).
    rf_channel_index: [usize; 2],
    rf_channel_data: [[u32; 2]; 14],
    /// Current channel (1..=14), 0 while the RF chip is not tuned. Frames are
    /// only exchanged with consoles on the same channel.
    cur_channel: u8,

    random: u16,
    us_counter: u64,
    us_compare: u64,
    block_beacon_irq14: bool,
    cmd_counter: u32,
    /// Negative while the transceiver is powering up (µs left).
    us_until_power_on: i32,

    /// Wi-Fi clock (µs since power-on); the timestamp frames are sent with.
    us_timestamp: u64,

    tx_slots: [TxSlot; 6],
    /// Frame being handed to the link (TX header + 802.11 frame).
    tx_buffer: Vec<u8>,
    /// Bit 0: receiving, bit 1: transmitting.
    com_status: u32,
    tx_cur_slot: Option<usize>,
    /// µs since the last RX poll.
    rx_counter: u32,

    /// Frame being received (RX header + 802.11 frame).
    rx_buffer: Vec<u8>,
    rx_buffer_ptr: usize,
    rx_time: i32,
    rx_halfword_time_mask: u32,

    /// MP host: time until the next client's reply slot, clients still to
    /// reply, clients that have not replied, and the replies themselves.
    mp_reply_timer: i32,
    mp_client_mask: u16,
    mp_client_fail: u16,
    mp_client_replies: Vec<u8>,
    /// Sequence number of the last MP CMD received (duplicates are dropped).
    mp_last_seqno: u16,

    /// A beacon slot is armed (this console hosts a MP session).
    is_mp: bool,
    /// This console is associated to a MP host and runs in lockstep with it.
    is_mp_client: bool,
    /// MP client: how far the clock may run before waiting for the host.
    next_sync: u64,
    /// MP client: when the pending host frame is to be received.
    rx_timestamp: u64,

    /// POWCNT2 bit 1.
    enabled: bool,
    /// `enabled` and W_POWER_US bit 0 clear: the timer runs.
    power_on: bool,
    /// ARM7 cycles not yet turned into timer steps.
    cycle_acc: u64,
    /// Rising edge of `W_IF & W_IE`, taken by [`Self::take_irq`].
    irq_pending: bool,

    /// The other consoles, if any.
    link: Option<Box<dyn MpLink>>,
    /// `link.begin()` was called and not yet matched by `end()`.
    link_begun: bool,
}

impl Default for WiFi {
    fn default() -> Self {
        Self::new()
    }
}

impl WiFi {
    /// Creates a powered-off controller with DS (not Lite) reset values.
    pub fn new() -> Self {
        let mut wifi = Self {
            io: [0; 0x800],
            ram: vec![0; 0x2000],
            bb_regs: [0; 0x100],
            bb_read_only: [false; 0x100],
            rf_regs: [0; 0x40],
            rf_version: 3,
            rf_channel_index: [0; 2],
            rf_channel_data: [[0; 2]; 14],
            cur_channel: 0,
            random: 1,
            us_counter: 0,
            us_compare: 0,
            block_beacon_irq14: false,
            cmd_counter: 0,
            us_until_power_on: 0,
            us_timestamp: 0,
            tx_slots: [TxSlot::default(); 6],
            tx_buffer: vec![0; 0x2000],
            com_status: 0,
            tx_cur_slot: None,
            rx_counter: 0,
            rx_buffer: vec![0; 0x1000],
            rx_buffer_ptr: 0,
            rx_time: 0,
            rx_halfword_time_mask: u32::MAX,
            mp_reply_timer: 0,
            mp_client_mask: 0,
            mp_client_fail: 0,
            mp_client_replies: vec![0; 15 * 1024],
            mp_last_seqno: 0xFFFF,
            is_mp: false,
            is_mp_client: false,
            next_sync: 0,
            rx_timestamp: 0,
            enabled: false,
            power_on: false,
            cycle_acc: 0,
            irq_pending: false,
            link: None,
            link_begun: false,
        };
        wifi.reset_state(0xFF, 3);
        wifi
    }

    /// Resets the controller (melonDS `Wifi::Reset`) from the firmware
    /// header: console type (0x1D), RF chip type (0x40) and the RF channel
    /// tables. The link, if any, is kept.
    pub fn reset(&mut self, firmware: &Firmware) {
        let fw = &firmware.raw_firmware;
        let byte = |offset: usize| fw.get(offset).copied().unwrap_or(0);
        let rf_version = byte(0x40);
        self.reset_state(byte(0x1D), rf_version);

        if rf_version == 3 {
            // Type3Config: RFIndex1 at 0x116, RFData1[14], RFIndex2 at 0x125,
            // RFData2[14].
            self.rf_channel_index = [usize::from(byte(0x116)), usize::from(byte(0x125))];
            for (i, data) in self.rf_channel_data.iter_mut().enumerate() {
                *data = [u32::from(byte(0x117 + i)), u32::from(byte(0x126 + i))];
            }
        } else {
            // Type2Config: InitialRF56Values at 0xF2, 6 bytes per channel
            // (two 18-bit RF register values).
            let rf56 = |i: usize| u32::from(byte(0xF2 + i));
            self.rf_channel_index = [(rf56(2) >> 2) as usize, (rf56(5) >> 2) as usize];
            for (i, data) in self.rf_channel_data.iter_mut().enumerate() {
                let at = i * 6;
                *data = [
                    rf56(at) | (rf56(at + 1) << 8) | ((rf56(at + 2) & 3) << 16),
                    rf56(at + 3) | (rf56(at + 4) << 8) | ((rf56(at + 5) & 3) << 16),
                ];
            }
        }
    }

    /// Register/RAM/state part of [`Self::reset`].
    ///
    /// `console_type` is firmware byte 0x1D (0xFF = DS, 0x57 = iQue DS,
    /// anything else is treated as a DS Lite) and `rf_version` firmware byte
    /// 0x40 (RF chip type).
    fn reset_state(&mut self, console_type: u8, rf_version: u8) {
        self.io.fill(0);
        self.ram.fill(0);

        self.bb_regs = [0; 0x100];
        self.bb_read_only = [false; 0x100];
        let fixed: &[(usize, u8)] = &[
            (0x00, 0x6D),
            (0x0D, 0),
            (0x0E, 0),
            (0x0F, 0),
            (0x10, 0),
            (0x11, 0),
            (0x12, 0),
            (0x16, 0),
            (0x17, 0),
            (0x18, 0),
            (0x19, 0),
            (0x1A, 0),
            (0x27, 0),
            (0x4D, 0),
            (0x5D, 0x01),
            (0x5E, 0),
            (0x5F, 0),
            (0x60, 0),
            (0x61, 0),
            (0x64, 0xFF),
            (0x66, 0),
        ];
        for &(id, val) in fixed {
            self.bb_regs[id] = val;
            self.bb_read_only[id] = true;
        }
        for id in 0x69..0x100 {
            self.bb_read_only[id] = true;
        }

        self.rf_regs = [0; 0x40];
        self.rf_version = rf_version;
        self.rf_channel_index = [0; 2];
        self.rf_channel_data = [[0; 2]; 14];
        self.cur_channel = 0;

        self.io[W_ID / 2] = match console_type {
            0xFF | 0x57 => 0x1440,
            _ => 0xC340,
        };
        for i in 0..3 {
            self.io[W_MACADDR_0 / 2 + i] = 0xFFFF;
            self.io[W_BSSID_0 / 2 + i] = 0xFFFF;
        }
        self.io[W_POWER_US / 2] = 0x0001;

        self.random = 1;
        self.us_counter = 0;
        self.us_compare = 0;
        self.block_beacon_irq14 = false;
        self.cmd_counter = 0;
        self.us_until_power_on = 0;
        self.us_timestamp = 0;

        self.tx_slots = [TxSlot::default(); 6];
        self.tx_buffer.fill(0);
        self.com_status = 0;
        self.tx_cur_slot = None;
        self.rx_counter = 0;

        self.rx_buffer.fill(0);
        self.rx_buffer_ptr = 0;
        self.rx_time = 0;
        self.rx_halfword_time_mask = u32::MAX;

        self.mp_reply_timer = 0;
        self.mp_client_mask = 0;
        self.mp_client_fail = 0;
        self.mp_client_replies.fill(0);
        self.mp_last_seqno = 0xFFFF;

        self.is_mp = false;
        self.is_mp_client = false;
        self.next_sync = 0;
        self.rx_timestamp = 0;

        self.enabled = false;
        self.cycle_acc = 0;
        self.irq_pending = false;
        self.update_power_on();
    }

    /// Connects the controller to other consoles (`None`: nobody in range).
    /// Takes effect immediately, also while the controller is powered.
    pub fn set_link(&mut self, link: Option<Box<dyn MpLink>>) {
        self.end_link();
        self.link = link;
        if self.power_on {
            self.begin_link();
        }
    }

    /// Whether a link is attached.
    pub const fn has_link(&self) -> bool {
        self.link.is_some()
    }

    /// Current channel (1..=14), or 0 while the RF chip is not tuned.
    pub const fn channel(&self) -> u8 {
        self.cur_channel
    }

    /// MAC address the wireless library programmed (`W_MACADDR`).
    pub const fn mac_address(&self) -> [u8; 6] {
        self.port_mac(W_MACADDR_0)
    }

    /// `W_AID_LOW`: non-zero once associated to a MP host.
    pub const fn aid(&self) -> u16 {
        self.port(W_AID_LOW)
    }

    /// This console is hosting (beacon slot armed) / is an associated client.
    pub const fn mp_state(&self) -> (bool, bool) {
        (self.is_mp, self.is_mp_client)
    }

    /// Start of an emulated video frame: lets the link pump its transport.
    pub fn new_frame(&mut self) {
        if let Some(link) = self.link.as_mut() {
            link.new_frame();
        }
    }

    fn begin_link(&mut self) {
        if let Some(link) = self.link.as_mut()
            && !self.link_begun
        {
            link.begin();
            self.link_begun = true;
        }
    }

    fn end_link(&mut self) {
        if let Some(link) = self.link.as_mut()
            && self.link_begun
        {
            link.end();
        }
        self.link_begun = false;
    }

    /// I/O port at byte `offset` (0x000..0xFFF).
    const fn port(&self, offset: usize) -> u16 {
        self.io[offset / 2]
    }

    /// Mutable I/O port at byte `offset`.
    const fn port_mut(&mut self, offset: usize) -> &mut u16 {
        &mut self.io[offset / 2]
    }

    /// Six bytes of I/O ports at `offset` (a MAC address / BSSID).
    const fn port_mac(&self, offset: usize) -> [u8; 6] {
        let [a, b] = self.port(offset).to_le_bytes();
        let [c, d] = self.port(offset + 2).to_le_bytes();
        let [e, f] = self.port(offset + 4).to_le_bytes();
        [a, b, c, d, e, f]
    }

    /// Little-endian halfword of Wi-Fi RAM (wrapping at 8 KB).
    fn ram_u16(&self, addr: usize) -> u16 {
        u16::from_le_bytes([self.ram[addr & 0x1FFF], self.ram[(addr + 1) & 0x1FFF]])
    }

    /// Stores a little-endian halfword into Wi-Fi RAM (wrapping at 8 KB).
    fn set_ram_u16(&mut self, addr: usize, value: u16) {
        let [lo, hi] = value.to_le_bytes();
        self.ram[addr & 0x1FFF] = lo;
        self.ram[(addr + 1) & 0x1FFF] = hi;
    }

    /// POWCNT2 bit 1 (Wi-Fi power) changed.
    pub fn set_power_cnt(&mut self, enabled: bool) {
        self.enabled = enabled;
        self.update_power_on();
    }

    /// Recomputes whether the controller is clocked (POWCNT2 bit 1,
    /// W_POWER_US bit 0); joins / leaves the link on the edges (melonDS
    /// `MP_Begin` / `MP_End`).
    fn update_power_on(&mut self) {
        let on = self.enabled && (self.port(W_POWER_US) & 1) == 0;
        if on != self.power_on {
            self.power_on = on;
            self.cycle_acc = 0;
            if on {
                self.begin_link();
            } else {
                self.end_link();
            }
        }
    }

    /// Returns `true` once per rising edge of the Wi-Fi IRQ line.
    pub const fn take_irq(&mut self) -> bool {
        let irq = self.irq_pending;
        self.irq_pending = false;
        irq
    }

    /// `W_IF & W_IE`.
    const fn active_irqs(&self) -> u16 {
        self.port(W_IF) & self.port(W_IE)
    }

    /// Flags an IRQ when `W_IF & W_IE` goes from 0 to non-zero.
    const fn check_irq(&mut self, old: u16) {
        if old == 0 && self.active_irqs() != 0 {
            self.irq_pending = true;
        }
    }

    /// Sets W_IF bit `irq` and checks the IRQ line.
    const fn set_irq(&mut self, irq: u32) {
        let old = self.active_irqs();
        *self.port_mut(W_IF) |= 1 << irq;
        self.check_irq(old);
    }

    /// IRQ 13 (post-beacon); powers the transceiver down in power-save mode 0.
    const fn set_irq13(&mut self) {
        self.set_irq(13);
        if (self.port(W_MODE_WEP) & 7) == 0 && (self.port(W_POWER_TX) & 2) == 0 {
            self.update_power_status(-1);
        }
    }

    /// `source`: 0 = US_COMPARE, 1 = beacon count, 2 = forced.
    fn set_irq14(&mut self, source: u32) {
        if source != 2 {
            *self.port_mut(W_BEACON_COUNT1) = self.port(W_BEACONINT);
        }
        if self.block_beacon_irq14 && source == 1 {
            return;
        }
        if self.port(W_US_COMPARECNT) & 1 == 0 {
            return;
        }
        self.set_irq(14);

        *self.port_mut(W_BEACON_COUNT2) = 0xFFFF;
        *self.port_mut(W_TXREQ_READ) &= 0xFFF2;

        if self.port(W_TXBUF_BEACON) & 0x8000 != 0 {
            self.start_tx_beacon();
        }

        if self.port(W_LISTENCOUNT) == 0 {
            *self.port_mut(W_LISTENCOUNT) = self.port(W_LISTENINT);
        }
        let count = self.port(W_LISTENCOUNT).wrapping_sub(1);
        *self.port_mut(W_LISTENCOUNT) = count;
    }

    /// IRQ 15 (pre-beacon); wakes the transceiver when W_POWER_TX bit 0 is set.
    const fn set_irq15(&mut self) {
        self.set_irq(15);
        if self.port(W_POWER_TX) & 1 != 0 {
            self.update_power_status(1);
        }
    }

    /// W_RF_STATUS / W_RF_PINS for a transceiver state.
    ///
    /// 1 = waiting for frames, 3 = TX, 5 = MP host waiting for replies,
    /// 6 = RX, 8 = MP reply / ACK being sent, 9 = idle.
    const fn set_status(&mut self, status: u16) {
        const RF_PINS: [u16; 10] = [0x04, 0x84, 0, 0x46, 0, 0x84, 0x87, 0, 0x46, 0x04];
        *self.port_mut(W_RF_STATUS) = status;
        *self.port_mut(W_RF_PINS) = RF_PINS[status as usize];
    }

    /// Transceiver power management (melonDS `UpdatePowerStatus`).
    /// `power`: 1 = wake up, 0 = re-evaluate, -1 = power down.
    const fn update_power_status(&mut self, mut power: i32) {
        let mut cur = 0;
        if self.port(W_TRX_POWER) == 1 {
            cur |= 1;
        }
        if self.port(W_POWERSTATE) & (1 << 9) == 0 {
            cur |= 2;
        }
        let mut req = cur;

        if self.port(W_POWERFORCE) & (1 << 15) != 0 {
            req = if self.port(W_POWERFORCE) & 1 != 0 {
                0
            } else {
                3
            };
        } else if self.port(W_MODE_RST) & 1 == 0 {
            req = 0;
        } else {
            if power == 0 {
                if self.port(W_POWERSTATE) & 0x0202 == 0x0202 {
                    power = 1;
                } else if self.port(W_POWERSTATE) & 0x0201 == 0x0001 {
                    power = -1;
                }
            }
            if power == -1 && self.port(W_POWERDOWNCTRL) & 1 != 0 {
                power = 0;
            }
            if power == 1 {
                req = 3;
            } else if power == -1 {
                req = if self.port(W_POWERDOWNCTRL) != 0 {
                    3
                } else {
                    0
                };
            } else if self.port(W_POWERDOWNCTRL) & 2 != 0 {
                req = 3;
            }
        }

        if req == cur {
            return;
        }

        if req & 1 != 0 {
            if cur & 1 == 0 {
                *self.port_mut(W_TRX_POWER) = 1;
                self.set_status(1);
            }
        } else {
            // Signal the transceiver is going to turn off; it does once the
            // transfer in progress (if any) is over.
            *self.port_mut(W_TRX_POWER) = 2;
            if self.com_status == 0 {
                *self.port_mut(W_TRX_POWER) = 0;
                self.set_status(9);
            }
        }

        if req & 2 != 0 {
            *self.port_mut(W_POWERSTATE) |= 1 << 8;
            if cur & 2 == 0 && self.us_until_power_on == 0 {
                self.us_until_power_on = -2048;
                self.set_irq(11);
            }
        } else {
            let state = (self.port(W_POWERSTATE) & !((1 << 0) | (1 << 8))) | (1 << 9);
            *self.port_mut(W_POWERSTATE) = state;
            self.us_until_power_on = 0;
        }
    }

    /// Advances the controller by `cycles` ARM7 bus cycles.
    pub fn run(&mut self, cycles: u32) {
        if !self.power_on {
            return;
        }
        self.cycle_acc += u64::from(cycles) * 1_000_000;
        let step = ARM7_HZ * u64::from(US_STEP);
        while self.cycle_acc >= step {
            self.cycle_acc -= step;
            self.us_timer();
        }
    }

    /// One `US_STEP` µs tick (melonDS `USTimer`).
    fn us_timer(&mut self) {
        self.us_timestamp += u64::from(US_STEP);

        // A MP client runs in lockstep with its host: frames from the host
        // are delivered at the timestamp they were sent with, and the clock
        // waits for the host at `next_sync`.
        if self.is_mp_client && self.com_status == 0 {
            if self.rx_timestamp != 0 && self.us_timestamp >= self.rx_timestamp {
                self.rx_timestamp = 0;
                self.start_rx();
            }
            if self.us_timestamp >= self.next_sync {
                self.check_rx(rx::RxSource::Host);
            }
        }

        if self.us_until_power_on < 0 {
            self.us_until_power_on += US_STEP as i32;
            if self.us_until_power_on >= 0 {
                self.us_until_power_on = 0;
                *self.port_mut(W_POWERSTATE) = 0;
                self.set_status(1);
                self.update_power_status(0);
            }
        }

        if self.port(W_US_COUNTCNT) != 0 {
            self.us_counter += u64::from(US_STEP);
            let uspart = self.us_counter & 0x3FF;

            if self.port(W_US_COMPARECNT) != 0 {
                let beacon_us = (u64::from(self.port(W_BEACON_COUNT1)) << 10) | (0x3FF - uspart);
                if beacon_us & STEP_MASK == u64::from(self.port(W_PRE_BEACON)) & STEP_MASK {
                    self.set_irq15();
                }
            }

            if uspart & STEP_MASK == 0 {
                self.ms_timer();
            }
        }

        if self.port(W_CMD_COUNTCNT) & 1 != 0 {
            self.cmd_counter = self.cmd_counter.saturating_sub(US_STEP);
        }

        let free = self.port(W_CONTENTFREE);
        *self.port_mut(W_CONTENTFREE) = free.saturating_sub(US_STEP as u16);

        if self.com_status == 0 {
            let txbusy = self.port(W_TXBUSY);
            if txbusy != 0 {
                if self.port(W_POWERSTATE) & (1 << 9) != 0 {
                    self.tx_cur_slot = None;
                } else {
                    self.tx_cur_slot = Self::next_tx_slot(txbusy);
                    if self.tx_cur_slot.is_some() {
                        self.com_status = 0x2;
                    }
                }
            } else {
                if (!self.is_mp_client || self.us_timestamp > self.next_sync)
                    && self.rx_counter & 0x1FF & STEP_MASK as u32 == 0
                {
                    self.check_rx(rx::RxSource::Any);
                }
                self.rx_counter = self.rx_counter.wrapping_add(US_STEP);
            }
        }

        if self.com_status & 0x2 != 0
            && let Some(slot) = self.tx_cur_slot
            && self.process_tx(slot)
        {
            if self.port(W_POWERSTATE) & (1 << 9) != 0 {
                *self.port_mut(W_TXBUSY) = 0;
                *self.port_mut(W_TRX_POWER) = 0;
                self.set_status(9);
            }
            // Transfer finished: see if there is another slot to send.
            self.tx_cur_slot = Self::next_tx_slot(self.port(W_TXBUSY));
            if self.tx_cur_slot.is_none() {
                self.com_status = 0;
                self.rx_counter = 0;
            }
        }

        if self.com_status & 0x1 != 0 {
            self.rx_tick();
        }
    }

    /// The busy TX slot to service next (reply, beacon, LOC3, LOC2, CMD,
    /// LOC1).
    const fn next_tx_slot(txbusy: u16) -> Option<usize> {
        if txbusy & 0x0080 != 0 {
            Some(5)
        } else if txbusy & 0x0010 != 0 {
            Some(4)
        } else if txbusy & 0x0008 != 0 {
            Some(3)
        } else if txbusy & 0x0004 != 0 {
            Some(2)
        } else if txbusy & 0x0002 != 0 {
            Some(1)
        } else if txbusy & 0x0001 != 0 {
            Some(0)
        } else {
            None
        }
    }

    /// Once per 1024 µs of W_US_COUNT (melonDS `MSTimer`).
    fn ms_timer(&mut self) {
        if self.port(W_US_COMPARECNT) != 0 && (self.us_counter & !0x3FF) == self.us_compare {
            self.block_beacon_irq14 = false;
            self.set_irq14(0);
        }

        if self.port(W_BEACON_COUNT1) != 0 {
            let count = self.port(W_BEACON_COUNT1) - 1;
            *self.port_mut(W_BEACON_COUNT1) = count;
            if count == 0 {
                self.set_irq14(1);
            }
        }
        if self.port(W_BEACON_COUNT1) == 0 {
            *self.port_mut(W_BEACON_COUNT1) = self.port(W_BEACONINT);
        }

        if self.port(W_BEACON_COUNT2) != 0 {
            let count = self.port(W_BEACON_COUNT2) - 1;
            *self.port_mut(W_BEACON_COUNT2) = count;
            if count == 0 {
                self.set_irq13();
            }
        }
    }

    /// Re-derives the channel from the two channel-selecting RF registers
    /// (melonDS `ChangeChannel`).
    fn change_channel(&mut self) {
        let val1 = self.rf_regs[self.rf_channel_index[0] & 0x3F];
        let val2 = self.rf_regs[self.rf_channel_index[1] & 0x3F];
        self.cur_channel = self
            .rf_channel_data
            .iter()
            .position(|&[a, b]| a == val1 && b == val2)
            .map_or(0, |i| i as u8 + 1);
        #[cfg(feature = "log")]
        log::debug!("wifi: channel {} ({val1:05X}:{val2:05X})", self.cur_channel);
    }

    /// RF chip serial transfer started by a W_RF_DATA2 write.
    fn rf_transfer(&mut self) {
        let written = if self.rf_version == 3 {
            let id = ((self.port(W_RF_DATA1) >> 8) & 0x3F) as usize;
            match self.port(W_RF_DATA2) & 0xF {
                6 => {
                    let data = (self.port(W_RF_DATA1) & 0xFF00) | (self.rf_regs[id] & 0xFF) as u16;
                    *self.port_mut(W_RF_DATA1) = data;
                    None
                }
                5 => {
                    self.rf_regs[id] = u32::from(self.port(W_RF_DATA1) & 0xFF);
                    Some(id)
                }
                _ => None,
            }
        } else {
            let id = ((self.port(W_RF_DATA2) >> 2) & 0x1F) as usize;
            if self.port(W_RF_DATA2) & 0x0080 != 0 {
                let data = self.rf_regs[id];
                *self.port_mut(W_RF_DATA1) = data as u16;
                let hi = (self.port(W_RF_DATA2) & 0xFFFC) | ((data >> 16) & 3) as u16;
                *self.port_mut(W_RF_DATA2) = hi;
                None
            } else {
                self.rf_regs[id] =
                    u32::from(self.port(W_RF_DATA1)) | (u32::from(self.port(W_RF_DATA2) & 3) << 16);
                Some(id)
            }
        };
        if let Some(id) = written
            && self.rf_channel_index.contains(&id)
        {
            self.change_channel();
        }
    }

    /// 16-bit read from `0x04800000..0x0480FFFF`.
    pub fn read(&mut self, address: u32) -> u16 {
        let addr = (address & 0x7FFE) as usize;
        if (0x4000..0x6000).contains(&addr) {
            return self.ram_u16(addr & 0x1FFE);
        }
        if (0x2000..0x4000).contains(&addr) {
            return 0xFFFF;
        }
        let active = addr < 0x1000;
        let port = addr & 0xFFF;

        match port {
            W_RANDOM => {
                self.random =
                    (self.random & 1) ^ (((self.random & 0x3FF) << 1) | (self.random >> 10));
                self.random
            }
            W_PREAMBLE => self.port(W_PREAMBLE) & 3,
            W_US_COUNT0 => self.us_counter as u16,
            W_US_COUNT1 => (self.us_counter >> 16) as u16,
            W_US_COUNT2 => (self.us_counter >> 32) as u16,
            W_US_COUNT3 => (self.us_counter >> 48) as u16,
            W_US_COMPARE0 => self.us_compare as u16,
            W_US_COMPARE1 => (self.us_compare >> 16) as u16,
            W_US_COMPARE2 => (self.us_compare >> 32) as u16,
            W_US_COMPARE3 => (self.us_compare >> 48) as u16,
            W_CMD_COUNT => self.cmd_counter.div_ceil(10) as u16,
            W_BB_READ => {
                if self.port(W_BB_CNT) & 0xF000 == 0x6000 {
                    u16::from(self.bb_regs[(self.port(W_BB_CNT) & 0xFF) as usize])
                } else {
                    0
                }
            }
            W_BB_BUSY | W_RF_BUSY => 0,
            W_RXBUF_RD_DATA => {
                if active {
                    self.read_rx_buffer();
                }
                self.port(W_RXBUF_RD_DATA)
            }
            W_TXBUSY => self.port(W_TXBUSY) & 0x001F,
            W_CMD_STAT0..=W_CMD_STAT7 => {
                let v = self.port(port);
                *self.port_mut(port) = 0;
                v
            }
            _ => self.port(port),
        }
    }

    /// W_RXBUF_RD_DATA read: fetch one halfword from the RX ring buffer.
    fn read_rx_buffer(&mut self) {
        let mut rd = self.port(W_RXBUF_RD_ADDR) as usize & 0x1FFE;
        let ret = self.ram_u16(rd);
        let begin = self.port(W_RXBUF_BEGIN) as usize & 0x1FFE;
        let end = self.port(W_RXBUF_END) as usize & 0x1FFE;

        rd += 2;
        if rd == end {
            rd = begin;
        }
        if rd == self.port(W_RXBUF_GAP) as usize {
            rd += (self.port(W_RXBUF_GAPDISP) as usize) << 1;
            if rd >= end {
                rd = rd + begin - end;
            }
            if self.port(W_ID) == 0xC340 {
                *self.port_mut(W_RXBUF_GAPDISP) = 0;
            }
        }
        *self.port_mut(W_RXBUF_RD_ADDR) = (rd & 0x1FFE) as u16;
        *self.port_mut(W_RXBUF_RD_DATA) = ret;

        if self.port(W_RXBUF_COUNT) > 0 {
            let count = self.port(W_RXBUF_COUNT) - 1;
            *self.port_mut(W_RXBUF_COUNT) = count;
            if count == 0 {
                self.set_irq(9);
            }
        }
    }

    /// 16-bit write to `0x04800000..0x0480FFFF`.
    pub fn write(&mut self, address: u32, value: u16) {
        let addr = (address & 0x7FFE) as usize;
        if (0x4000..0x6000).contains(&addr) {
            self.set_ram_u16(addr & 0x1FFE, value);
            return;
        }
        if (0x2000..0x4000).contains(&addr) {
            return;
        }
        let port = addr & 0xFFF;
        let mut val = value;

        match port {
            W_MODE_RST => {
                self.write_mode_reset(val);
                return;
            }
            W_MODE_WEP => {
                val &= 0x007F;
                *self.port_mut(W_MODE_WEP) = val;
                if self.port(W_POWER_TX) & 2 != 0 {
                    self.apply_power_down_mode();
                    if val & 7 != 3 {
                        *self.port_mut(W_POWERSTATE) &= 0x0300;
                    }
                    self.update_power_status(0);
                }
                return;
            }
            W_IE => {
                let old = self.active_irqs();
                *self.port_mut(W_IE) = val;
                self.check_irq(old);
                return;
            }
            W_IF => {
                *self.port_mut(W_IF) &= !val;
                return;
            }
            W_IF_SET => {
                let old = self.active_irqs();
                *self.port_mut(W_IF) |= val & 0xFBFF;
                self.check_irq(old);
                return;
            }
            W_AID_LOW => val &= 0x000F,
            W_AID_FULL => val &= 0x07FF,
            W_POWER_US => {
                *self.port_mut(W_POWER_US) = val & 3;
                self.update_power_on();
                return;
            }
            W_POWER_TX => {
                *self.port_mut(W_POWER_TX) = val & 3;
                if val & 2 != 0 {
                    self.apply_power_down_mode();
                    self.update_power_status(0);
                }
                return;
            }
            W_POWERSTATE => {
                if self.port(W_MODE_WEP) & 7 != 3 {
                    return;
                }
                let mut v = (self.port(W_POWERSTATE) & 0x0300) | (val & 3);
                if v & 0x0300 == 0x0200 {
                    v &= !1;
                } else {
                    v &= !2;
                }
                if v & (1 << 9) == 0 {
                    v &= !(1 << 8);
                }
                *self.port_mut(W_POWERSTATE) = v;
                self.update_power_status(0);
                return;
            }
            W_POWERFORCE => {
                *self.port_mut(W_POWERFORCE) = val & 0x8001;
                self.update_power_status(0);
                return;
            }
            W_POWERDOWNCTRL => {
                *self.port_mut(W_POWERDOWNCTRL) = val & 3;
                if self.port(W_POWER_TX) & 2 != 0 {
                    self.apply_power_down_mode();
                }
                self.update_power_status(0);
                return;
            }
            W_US_COUNTCNT => val &= 1,
            W_US_COMPARECNT => {
                if val & 2 != 0 {
                    self.set_irq14(2);
                }
                val &= 1;
            }
            W_US_COUNT0 => return self.set_us_counter(0, val),
            W_US_COUNT1 => return self.set_us_counter(16, val),
            W_US_COUNT2 => return self.set_us_counter(32, val),
            W_US_COUNT3 => return self.set_us_counter(48, val),
            W_US_COMPARE0 => {
                self.us_compare = (self.us_compare & !0xFFFF) | u64::from(val & 0xFC00);
                if val & 1 != 0 {
                    self.block_beacon_irq14 = true;
                }
                return;
            }
            W_US_COMPARE1 => return self.set_us_compare(16, val),
            W_US_COMPARE2 => return self.set_us_compare(32, val),
            W_US_COMPARE3 => return self.set_us_compare(48, val),
            W_CMD_COUNT => {
                self.cmd_counter = u32::from(val) * 10;
                return;
            }
            W_BB_CNT => {
                *self.port_mut(W_BB_CNT) = val;
                if val & 0xF000 == 0x5000 {
                    let id = (val & 0xFF) as usize;
                    if !self.bb_read_only[id] {
                        self.bb_regs[id] = self.port(W_BB_WRITE) as u8;
                    }
                }
                return;
            }
            W_RF_DATA2 => {
                *self.port_mut(W_RF_DATA2) = val;
                self.rf_transfer();
                return;
            }
            W_RF_CNT => val &= 0x413F,
            W_RXCNT => {
                if val & 0x0001 != 0 {
                    *self.port_mut(W_RXBUF_WRCSR) = self.port(W_RXBUF_WR_ADDR);
                }
                if val & 0x0080 != 0 {
                    *self.port_mut(W_TXBUF_REPLY2) = self.port(W_TXBUF_REPLY1);
                    *self.port_mut(W_TXBUF_REPLY1) = 0;
                }
                if val & 0x8000 != 0 {
                    self.fire_tx();
                }
                val &= 0xFF0E;
            }
            W_RXBUF_RD_DATA => {
                if self.port(W_RXBUF_COUNT) > 0 {
                    let count = self.port(W_RXBUF_COUNT) - 1;
                    *self.port_mut(W_RXBUF_COUNT) = count;
                    if count == 0 {
                        self.set_irq(9);
                    }
                }
                return;
            }
            W_RXBUF_RD_ADDR | W_RXBUF_GAP | W_TXBUF_WR_ADDR | W_TXBUF_GAP => val &= 0x1FFE,
            W_RXBUF_GAPDISP | W_RXBUF_COUNT | W_RXBUF_WR_ADDR | W_RXBUF_READCSR
            | W_TXBUF_GAPDISP | W_TXBUF_COUNT => val &= 0x0FFF,
            W_TXREQ_RESET => {
                *self.port_mut(W_TXREQ_READ) &= !val;
                return;
            }
            W_TXREQ_SET => {
                *self.port_mut(W_TXREQ_READ) |= val;
                self.fire_tx();
                return;
            }
            W_TXBUF_RESET => {
                for (bit, slot) in [
                    (0x0001, W_TXBUF_LOC1),
                    (0x0002, W_TXBUF_CMD),
                    (0x0004, W_TXBUF_LOC2),
                    (0x0008, W_TXBUF_LOC3),
                    (0x0040, W_TXBUF_REPLY2),
                    (0x0080, W_TXBUF_REPLY1),
                ] {
                    if val & bit != 0 {
                        *self.port_mut(slot) &= 0x7FFF;
                    }
                }
                val = 0;
            }
            W_TXBUF_WR_DATA => {
                self.write_tx_buffer(val);
                return;
            }
            W_TXBUF_BEACON => self.is_mp = val & 0x8000 != 0,
            W_TXBUF_CMD | W_TXBUF_LOC1 | W_TXBUF_LOC2 | W_TXBUF_LOC3 => {
                // A CMD slot cannot be (re)armed once CMD_COUNT ran out.
                if port == W_TXBUF_CMD && self.cmd_counter == 0 {
                    val = (val & 0x7FFF) | (self.port(W_TXBUF_CMD) & 0x8000);
                }
                *self.port_mut(port) = val;
                self.fire_tx();
                return;
            }
            // Read-only ports.
            W_ID | W_TRX_POWER | W_RANDOM | W_RXBUF_WRCSR | W_TXBUF_REPLY2 | W_TXREQ_READ
            | W_TXBUSY | W_TXSTAT | W_BB_READ | W_BB_BUSY | W_RF_BUSY | W_RF_PINS | 0x1A8
            | 0x1AC | 0x1C4 | W_TX_SEQNO | W_RF_STATUS | W_RXTX_ADDR => return,
            _ => {}
        }

        *self.port_mut(port) = val;
    }

    /// W_TXBUF_WR_DATA write: store one halfword into the TX buffer.
    fn write_tx_buffer(&mut self, val: u16) {
        let mut wr = self.port(W_TXBUF_WR_ADDR) as usize & 0x1FFE;
        self.set_ram_u16(wr, val);
        wr += 2;
        if wr == self.port(W_TXBUF_GAP) as usize {
            wr += (self.port(W_TXBUF_GAPDISP) as usize) << 1;
        }
        *self.port_mut(W_TXBUF_WR_ADDR) = (wr & 0x1FFE) as u16;

        if self.port(W_TXBUF_COUNT) > 0 {
            let count = self.port(W_TXBUF_COUNT) - 1;
            *self.port_mut(W_TXBUF_COUNT) = count;
            if count == 0 {
                self.set_irq(8);
            }
        }
    }

    /// W_POWER_TX bit 1: W_MODE_WEP selects how W_POWERDOWNCTRL behaves.
    const fn apply_power_down_mode(&mut self) {
        match self.port(W_MODE_WEP) & 7 {
            1 => *self.port_mut(W_POWERDOWNCTRL) |= 2,
            2 => *self.port_mut(W_POWERDOWNCTRL) = 3,
            _ => {}
        }
    }

    /// W_MODE_RST: bit 0 starts/stops the MAC; bits 13/14 reset port groups.
    fn write_mode_reset(&mut self, val: u16) {
        let old = self.port(W_MODE_RST);
        *self.port_mut(W_MODE_RST) = val & 1;

        if old & 1 == 0 && val & 1 != 0 {
            *self.port_mut(0x27C) = 0x0005;
            self.update_power_status(0);
        } else if old & 1 != 0 && val & 1 == 0 {
            *self.port_mut(0x27C) = 0x000A;
            self.update_power_status(0);
        }

        if val & 0x2000 != 0 {
            *self.port_mut(W_RXBUF_WR_ADDR) = 0;
            *self.port_mut(W_CMD_TOTALTIME) = 0;
            *self.port_mut(W_CMD_REPLYTIME) = 0;
            *self.port_mut(0x1A4) = 0;
            *self.port_mut(0x278) = 0x000F;
        }
        if val & 0x4000 != 0 {
            for (port, v) in [
                (W_MODE_WEP, 0),
                (W_TXSTATCNT, 0),
                (0x00A, 0),
                (W_MACADDR_0, 0),
                (W_MACADDR_0 + 2, 0),
                (W_MACADDR_0 + 4, 0),
                (W_BSSID_0, 0),
                (W_BSSID_0 + 2, 0),
                (W_BSSID_0 + 4, 0),
                (W_AID_LOW, 0),
                (W_AID_FULL, 0),
                (W_TX_RETRYLIMIT, 0x0707),
                (0x02E, 0),
                (W_RXBUF_BEGIN, 0x4000),
                (W_RXBUF_END, 0x4800),
                (W_TXBUF_TIM, 0),
                (W_PREAMBLE, 0x0001),
                (W_RXFILTER, 0x0401),
                (0x0D4, 0x0001),
                (W_RXFILTER2, 0x0008),
                (0x0EC, 0x3F03),
                (W_TX_HDR_CNT, 0),
                (0x198, 0),
                (0x1A2, 0x0001),
                (0x224, 0x0003),
                (0x230, 0x0047),
            ] {
                *self.port_mut(port) = v;
            }
        }
    }

    /// Writes one 16-bit part of W_US_COUNT.
    fn set_us_counter(&mut self, shift: u32, val: u16) {
        self.us_counter = (self.us_counter & !(0xFFFF << shift)) | (u64::from(val) << shift);
    }

    /// Writes one 16-bit part of W_US_COMPARE.
    fn set_us_compare(&mut self, shift: u32, val: u16) {
        self.us_compare = (self.us_compare & !(0xFFFF << shift)) | (u64::from(val) << shift);
    }
}

#[cfg(test)]
mod tests;
