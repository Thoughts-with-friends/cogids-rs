//! Transmit side: TX slots, the MP host sequence and MP client replies.
//!
//! ```text
//!  W_TXREQ_SET / W_TXBUF_LOCn / W_RXCNT bit 15 -> fire_tx()   picks a slot
//!  IRQ14 with W_TXBUF_BEACON armed              -> start_tx_beacon()
//!  MP CMD received (rx.rs)                      -> send_mp_reply()
//!    |
//!    v
//!  us_timer -> process_tx(slot), phase by phase:
//!    0  preamble done   IRQ7, frame handed to the link (tx_send_frame)
//!    1  transmit done   LOCn/beacon: IRQ1, slot freed
//!                       CMD: collect the clients' replies (link.recv_replies)
//!    2  replies window  each client's reply is received in its time slot
//!    3  ACK sent        IRQ12, CMD slot freed
//!    10/11              empty MP reply (client had nothing queued)
//!    13                 CMD timed out (CMD_COUNT too short)
//! ```
//!
//! Multiplayer host sequence (melonDS `Wifi.cpp`): preamble, IRQ7, data,
//! optional IRQ1, wait for the clients' replies
//! (`16 + (10 + CMD_REPLYTIME) * clients` µs), IRQ7, ACK, IRQ12.

use super::*;

/// Destination MACs of MP frames.
pub(super) const MP_CMD_MAC: [u8; 6] = [0x03, 0x09, 0xBF, 0x00, 0x00, 0x00];
pub(super) const MP_REPLY_MAC: [u8; 6] = [0x03, 0x09, 0xBF, 0x00, 0x00, 0x10];
pub(super) const MP_ACK_MAC: [u8; 6] = [0x03, 0x09, 0xBF, 0x00, 0x00, 0x03];

/// µs per byte at a slot's rate.
const fn us_per_byte(rate: u8) -> u32 {
    if rate == 2 { 4 } else { 8 }
}

impl WiFi {
    /// Preamble length in µs (long at 1 Mbit/s or when W_PREAMBLE bit 2 is
    /// clear).
    const fn preamble_len(&self, rate: u8) -> i32 {
        if rate != 1 && self.port(W_PREAMBLE) & 0x0004 != 0 {
            96
        } else {
            192
        }
    }

    /// The TX header's retry counter (byte 4) saturates at 0xFF.
    fn increment_tx_count(&mut self, slot: usize) {
        let addr = self.tx_slots[slot].addr;
        let count = self.ram[(addr + 4) & 0x1FFF];
        self.set_ram_u16(addr + 4, u16::from(count.saturating_add(1)));
    }

    /// Each client that failed to reply bumps its W_CMD_STAT counter.
    fn report_mp_reply_errors(&mut self, client_fail: u16) {
        for i in 1..16 {
            if client_fail & (1 << i) != 0 {
                let port = W_CMD_STAT0 + i;
                let shift = (port & 1) * 8;
                let word = self.port(port & !1);
                let count = ((word >> shift) as u8).wrapping_add(1);
                *self.port_mut(port & !1) = (word & !(0xFF << shift)) | (u16::from(count) << shift);
            }
        }
    }

    /// Hands the frame of `slot` to the link (melonDS `TXSendFrame`).
    /// `num`: 0/2/3 LOC1-3, 1 CMD, 4 beacon, 5 MP reply.
    fn tx_send_frame(&mut self, num: usize) {
        let TxSlot { addr, length, .. } = self.tx_slots[num];

        let no_seqno = if self.ram[(addr + 4) & 0x1FFF] != 0 {
            2
        } else {
            u32::from(num == 1 && self.port(W_TXBUF_CMD) & 0x4000 != 0)
        };
        if no_seqno == 0 {
            if self.port(W_TX_HDR_CNT) & (1 << 2) == 0 {
                self.set_ram_u16(addr + 0xC + 22, self.port(W_TX_SEQNO) << 4);
            }
            *self.port_mut(W_TX_SEQNO) = (self.port(W_TX_SEQNO) + 1) & 0x0FFF;
        }

        // WEP frames: no WEP processing, but some games want a non-zero FCS.
        let framectl = self.ram_u16(addr + 0xC);
        if framectl & (1 << 14) != 0 && self.port(W_WEPCNT) & (1 << 15) != 0 {
            let fcs = (addr + 0xC + usize::from(length)).wrapping_sub(7) & !1;
            for (i, b) in 0x2233_4466_u32.to_le_bytes().into_iter().enumerate() {
                self.ram[(fcs + i) & 0x1FFF] = b;
            }
        }

        let len = usize::from(length).min(0x1FF4_usize.saturating_sub(addr));
        let total = (12 + len).min(0x2000 - addr.min(0x2000));
        self.tx_buffer[..total].copy_from_slice(&self.ram[addr..addr + total]);
        if no_seqno == 2 {
            self.tx_buffer[0xD] |= 1 << 3; // frame control bit 11: retry
        }

        if self.cur_channel == 0 {
            return;
        }
        self.tx_buffer[9] = self.cur_channel;

        let ts = self.us_timestamp;
        match num {
            1 => {
                self.tx_buffer[12 + 24 + 2..12 + 24 + 4]
                    .copy_from_slice(&self.mp_client_mask.to_le_bytes());
            }
            4 => {
                self.tx_buffer[0xC + 24..0xC + 32].copy_from_slice(&self.us_counter.to_le_bytes());
            }
            5 => self.increment_tx_count(num),
            _ => {}
        }
        let aid = self.port(W_AID_LOW);
        let frame = &self.tx_buffer[..total];
        if let Some(link) = self.link.as_mut() {
            match num {
                1 => link.send_cmd(frame, ts),
                5 => link.send_reply(frame, ts, aid),
                _ => link.send_packet(frame, ts),
            }
        }
    }

    /// Loads slot `nslot` from W_TXBUF_LOC1/2/3 (`loc` 0..=2).
    fn start_tx_loc(&mut self, nslot: usize, loc: usize) {
        let addr = (usize::from(self.port(W_TXBUF_LOC1 + loc * 4)) & 0x0FFF) << 1;
        self.load_slot(nslot, addr);
        self.tx_slots[nslot].cur_phase = 0;
        self.tx_slots[nslot].cur_phase_time = self.preamble_len(self.tx_slots[nslot].rate);
    }

    /// Fills address, length and rate of a slot from its TX header.
    fn load_slot(&mut self, nslot: usize, addr: usize) {
        let length = self.ram_u16(addr + 0xA) & 0x3FFF;
        let rate = if self.ram[(addr + 8) & 0x1FFF] == 0x14 {
            2
        } else {
            1
        };
        let slot = &mut self.tx_slots[nslot];
        slot.valid = true;
        slot.addr = addr;
        slot.length = length;
        slot.rate = rate;
    }

    /// Starts the MP host CMD transfer (slot 1).
    fn start_tx_cmd(&mut self) {
        let addr = (usize::from(self.port(W_TXBUF_CMD)) & 0x0FFF) << 1;
        self.load_slot(1, addr);
        let TxSlot { length, rate, .. } = self.tx_slots[1];

        self.mp_client_mask = self.ram_u16(addr + 12 + 24 + 2) & self.mp_client_fail;
        self.mp_client_fail &= self.mp_client_mask;

        let per_byte = us_per_byte(rate);
        let preamble = self.preamble_len(rate);
        let mut duration = preamble as u32 + u32::from(length) * per_byte;
        duration +=
            112 + (10 + u32::from(self.port(W_CMD_REPLYTIME))) * self.mp_client_mask.count_ones();
        duration += 32 * per_byte;

        let slot = &mut self.tx_slots[1];
        if self.cmd_counter > duration + 100 {
            slot.cur_phase = 0;
            slot.cur_phase_time = preamble;
        } else {
            // Not enough CMD_COUNT left: fail once it runs out.
            slot.cur_phase = 13;
            slot.cur_phase_time = self.cmd_counter as i32 - 100;
        }

        // Starting a CMD transfer wakes the transceiver up.
        self.update_power_status(1);
    }

    /// Starts sending the beacon (slot 4) at IRQ14.
    pub(super) fn start_tx_beacon(&mut self) {
        let addr = (usize::from(self.port(W_TXBUF_BEACON)) & 0x0FFF) << 1;
        self.load_slot(4, addr);
        self.tx_slots[4].cur_phase = 0;
        self.tx_slots[4].cur_phase_time = self.preamble_len(self.tx_slots[4].rate);
        *self.port_mut(W_TXBUSY) |= 0x0010;
    }

    /// Starts the highest-priority requested slot that is armed and idle
    /// (melonDS `FireTX`).
    pub(super) fn fire_tx(&mut self) {
        if self.port(W_RXCNT) & 0x8000 == 0 {
            return;
        }
        let txbusy = self.port(W_TXBUSY);
        let txreq = self.port(W_TXREQ_READ);
        let mut txstart = 0;
        for (bit, port) in [
            (0x0001, W_TXBUF_LOC1),
            (0x0002, W_TXBUF_CMD),
            (0x0004, W_TXBUF_LOC2),
            (0x0008, W_TXBUF_LOC3),
        ] {
            if self.port(port) & 0x8000 != 0 {
                txstart |= bit;
            }
        }
        txstart &= txreq & !txbusy;
        *self.port_mut(W_TXBUSY) = txbusy | txstart;

        if txstart & 0x0008 != 0 {
            self.start_tx_loc(3, 2);
        } else if txstart & 0x0004 != 0 {
            self.start_tx_loc(2, 1);
        } else if txstart & 0x0002 != 0 {
            self.mp_client_fail = 0xFFFE;
            self.start_tx_cmd();
        } else if txstart & 0x0001 != 0 {
            self.start_tx_loc(0, 0);
        }
    }

    /// MP client with nothing queued: send an empty reply so the host does
    /// not wait for a timeout.
    fn send_mp_default_reply(&mut self) {
        if self.cur_channel == 0 {
            return;
        }
        let mut reply = [0_u8; 12 + 28];
        reply[0xA..0xC].copy_from_slice(&28_u16.to_le_bytes());
        reply[0x8] = 0x14;
        reply[0x9] = self.cur_channel;

        let bssid = self.port_mac(W_BSSID_0);
        let mac = self.port_mac(W_MACADDR_0);
        let mut put = |offset: usize, value: u16| {
            reply[0xC + offset..0xC + offset + 2].copy_from_slice(&value.to_le_bytes());
        };
        put(0x00, 0x0158);
        put(0x02, 0x00F0);
        put(0x10, 0x0903);
        put(0x12, 0x00BF);
        put(0x14, 0x1000);
        put(0x16, self.port(W_TX_SEQNO) << 4);
        reply[0xC + 0x04..0xC + 0x0A].copy_from_slice(&bssid);
        reply[0xC + 0x0A..0xC + 0x10].copy_from_slice(&mac);

        let (ts, aid) = (self.us_timestamp, self.port(W_AID_LOW));
        if let Some(link) = self.link.as_mut() {
            link.send_reply(&reply, ts, aid);
        }
    }

    /// MP client: answer the host's CMD from W_TXBUF_REPLY2 (or with an
    /// empty reply), timed to this client's slot in the reply window.
    pub(super) fn send_mp_reply(&mut self, client_time: u16, client_mask: u16) {
        // Mark the previous reply as sent.
        if self.port(W_TXBUF_REPLY2) & 0x8000 != 0 {
            let addr = self.tx_slots[5].addr;
            self.set_ram_u16(addr, 0x0001);
        }
        self.tx_slots[5].rate = 2;

        *self.port_mut(W_TXBUF_REPLY2) = self.port(W_TXBUF_REPLY1);
        *self.port_mut(W_TXBUF_REPLY1) = 0;

        if self.port(W_TXBUF_REPLY2) & 0x8000 == 0 {
            self.tx_slots[5].valid = false;
        } else {
            let addr = (usize::from(self.port(W_TXBUF_REPLY2)) & 0x0FFF) << 1;
            let length = self.ram_u16(addr + 0xA) & 0x3FFF;
            // A reply longer than the reply window is not sent at all.
            let duration = self.preamble_len(2) as u32 + u32::from(length) * 4;
            let slot = &mut self.tx_slots[5];
            slot.valid = duration <= u32::from(client_time);
            slot.addr = addr;
            slot.length = length;
        }

        if self.tx_slots[5].valid {
            self.tx_slots[5].cur_phase = 0;
            self.tx_send_frame(5);
        } else {
            self.tx_slots[5].cur_phase = 10;
            self.send_mp_default_reply();
        }

        let aid = u32::from(self.port(W_AID_LOW));
        let client_num = (1..aid).filter(|&i| client_mask & (1 << i) != 0).count() as i32;
        self.tx_slots[5].cur_phase_time =
            16 + (i32::from(client_time) + 10) * client_num + self.preamble_len(2);

        *self.port_mut(W_TXBUSY) |= 0x0080;
    }

    /// MP host: the ACK closing a CMD/reply round.
    fn send_mp_ack(&mut self, cmd_count: u16, client_fail: u16) {
        if self.cur_channel == 0 {
            return;
        }
        let rate = self.tx_slots[1].rate;
        let mut ack = [0_u8; 12 + 32];
        ack[0xA..0xC].copy_from_slice(&32_u16.to_le_bytes());
        ack[0x8] = if rate == 2 { 0x14 } else { 0xA };
        ack[0x9] = self.cur_channel;

        let bssid = self.port_mac(W_BSSID_0);
        let mac = self.port_mac(W_MACADDR_0);
        let seqno = self.port(W_TX_SEQNO) << 4;
        let mut put = |offset: usize, value: u16| {
            ack[0xC + offset..0xC + offset + 2].copy_from_slice(&value.to_le_bytes());
        };
        put(0x00, 0x0218);
        put(0x04, 0x0903);
        put(0x06, 0x00BF);
        put(0x08, 0x0300);
        put(0x16, seqno);
        put(0x18, cmd_count);
        put(0x1A, client_fail);
        ack[0xC + 0x0A..0xC + 0x10].copy_from_slice(&bssid);
        ack[0xC + 0x10..0xC + 0x16].copy_from_slice(&mac);

        // The TX header's first word tells the clients how far they may run
        // ahead before the next round (melonDS extension, see rx.rs).
        let runahead = if client_fail == 0 {
            let next_beacon = if self.port(W_TXBUSY) & 0x0010 != 0 {
                0
            } else {
                (u32::from(self.port(W_BEACON_COUNT1)).wrapping_sub(1) << 10)
                    .wrapping_add(0x400 - (self.us_counter & 0x3FF) as u32)
            };
            let mut runahead = i64::from(self.cmd_counter.min(next_beacon) as i32);
            if self.cmd_counter < 1000 {
                runahead -= 210;
            }
            (runahead - i64::from(32 * us_per_byte(rate))).max(0) as u32
        } else {
            self.preamble_len(rate) as u32
        };
        ack[..4].copy_from_slice(&runahead.to_le_bytes());

        let ts = self.us_timestamp;
        if let Some(link) = self.link.as_mut() {
            link.send_ack(&ack, ts);
        }
    }

    /// Advances the transfer of `num` by one step; `true` once it is done
    /// (melonDS `ProcessTX`).
    pub(super) fn process_tx(&mut self, num: usize) -> bool {
        self.tx_slots[num].cur_phase_time -= US_STEP as i32;
        let TxSlot {
            addr,
            length,
            rate,
            cur_phase,
            cur_phase_time,
            halfword_time_mask,
            ..
        } = self.tx_slots[num];

        if cur_phase_time > 0 {
            if cur_phase == 1 {
                if cur_phase_time as u32 & halfword_time_mask == 0 {
                    *self.port_mut(W_RXTX_ADDR) = self.port(W_RXTX_ADDR).wrapping_add(1);
                }
            } else if cur_phase == 2 {
                self.mp_reply_timer -= US_STEP as i32;
                if self.mp_reply_timer <= 0 && self.mp_client_mask != 0 {
                    // This client's reply slot: deliver its reply, if any.
                    let client = (self.mp_client_mask & !1).trailing_zeros() as usize;
                    let bit = 1 << client;
                    if self.mp_client_fail & bit == 0 {
                        self.mp_client_reply_rx(client);
                    }
                    self.mp_reply_timer += 10 + i32::from(self.port(W_CMD_REPLYTIME));
                    self.mp_client_mask &= !bit;
                }
            }
            return false;
        }

        match cur_phase {
            // Preamble done.
            0 => {
                self.set_irq(7);
                self.set_status(if num == 5 { 8 } else { 3 });

                let (len, mask) = if rate == 2 {
                    (u32::from(length) * 4, 0x7 & STEP_MASK as u32)
                } else {
                    (u32::from(length) * 8, 0xF & STEP_MASK as u32)
                };
                let slot = &mut self.tx_slots[num];
                slot.halfword_time_mask = mask;
                slot.cur_phase = 1;
                slot.cur_phase_time = len as i32;

                *self.port_mut(W_RXTX_ADDR) = (addr >> 1) as u16;
                if num != 5 {
                    self.tx_send_frame(num);
                }

                // A client sending a deauthentication leaves the session.
                if matches!(num, 0 | 2 | 3)
                    && self.ram_u16(addr + 0xC) & 0x00FF == 0x00C0
                    && self.is_mp_client
                {
                    self.is_mp = false;
                    self.is_mp_client = false;
                }
                false
            }
            // Preamble of an empty MP reply done.
            10 => {
                self.set_irq(7);
                self.set_status(8);
                let slot = &mut self.tx_slots[num];
                slot.cur_phase = 11;
                slot.cur_phase_time = 28 * 4;
                slot.halfword_time_mask = u32::MAX;
                false
            }
            // Transmit done.
            1 => self.tx_done(num, addr, rate),
            // Empty MP reply sent.
            11 => {
                *self.port_mut(W_TX_SEQNO) = (self.port(W_TX_SEQNO) + 1) & 0x0FFF;
                *self.port_mut(W_TXBUSY) &= !0x80;
                self.set_status(1);
                self.fire_tx();
                true
            }
            // MP host: reply window over, send the ACK.
            2 => {
                self.set_irq(7);
                self.set_status(8);
                *self.port_mut(W_RXTX_ADDR) = 0xFC0;
                let slot = &mut self.tx_slots[num];
                slot.cur_phase_time = 32 * us_per_byte(rate) as i32;
                slot.cur_phase = 3;

                self.report_mp_reply_errors(self.mp_client_fail);
                let cmd_count = self.cmd_counter.div_ceil(10) as u16;
                self.send_mp_ack(cmd_count, self.mp_client_fail);
                false
            }
            // MP host: ACK sent, round over.
            3 => {
                let fail = self.mp_client_fail;
                self.set_ram_u16(addr, if fail == 0 { 0x0001 } else { 0x0005 });
                // Which clients failed to reply.
                self.set_ram_u16(addr + 2, fail);
                if fail == 0 {
                    self.increment_tx_count(num);
                }
                *self.port_mut(W_TX_SEQNO) = (self.port(W_TX_SEQNO) + 1) & 0x0FFF;
                if self.port(W_TXSTATCNT) & 0x2000 != 0 {
                    *self.port_mut(W_TXSTAT) = 0x0B01;
                    self.set_irq(1);
                }
                self.finish_cmd()
            }
            // MP host: CMD_COUNT ran out.
            13 => {
                self.set_ram_u16(addr, 0x0005);
                *self.port_mut(W_TX_SEQNO) = (self.port(W_TX_SEQNO) + 1) & 0x0FFF;
                self.finish_cmd()
            }
            _ => false,
        }
    }

    /// End of a CMD transfer: free the slot, IRQ12, start the next one.
    fn finish_cmd(&mut self) -> bool {
        *self.port_mut(W_TXBUSY) &= !(1 << 1);
        *self.port_mut(W_TXBUF_CMD) &= 0x7FFF;
        self.set_status(1);
        self.set_irq(12);
        self.fire_tx();
        true
    }

    /// Phase 1 of [`Self::process_tx`]: the frame is out.
    fn tx_done(&mut self, num: usize, addr: usize, rate: u8) -> bool {
        // For the CMD and reply slots the status is written later.
        if num != 1 && num != 5 {
            self.set_ram_u16(addr, 0x0001);
        }
        self.ram[(addr + 5) & 0x1FFF] = 0;

        match num {
            1 => {
                if self.port(W_TXSTATCNT) & 0x4000 != 0 {
                    *self.port_mut(W_TXSTAT) = 0x0800;
                    self.set_irq(1);
                }
                self.set_status(5);
                self.mp_reply_timer = 16 + self.preamble_len(rate);

                if self.mp_client_mask != 0 {
                    let (ts, mask) = (self.us_timestamp, self.mp_client_mask);
                    if let Some(link) = self.link.as_mut() {
                        let received = link.recv_replies(&mut self.mp_client_replies, ts, mask);
                        self.mp_client_fail &= !received;
                    }
                }
                let window = 112
                    + (10 + i32::from(self.port(W_CMD_REPLYTIME)))
                        * self.mp_client_mask.count_ones() as i32;
                let slot = &mut self.tx_slots[1];
                slot.cur_phase = 2;
                slot.cur_phase_time = window;
                false
            }
            5 => {
                if self.port(W_TXSTATCNT) & 0x1000 != 0 {
                    *self.port_mut(W_TXSTAT) = 0x0401;
                    self.set_irq(1);
                }
                self.set_status(1);
                *self.port_mut(W_TXBUSY) &= !0x80;
                self.fire_tx();
                true
            }
            _ => {
                *self.port_mut(W_TXBUSY) &= !(1 << num);
                match num {
                    0 | 2 | 3 => {
                        let loc = num.saturating_sub(1);
                        *self.port_mut(W_TXSTAT) = 0x0001 | ((loc as u16) << 12);
                        self.set_irq(1);
                        *self.port_mut(W_TXBUF_LOC1 + loc * 4) &= 0x7FFF;
                    }
                    4 if self.port(W_TXSTATCNT) & 0x8000 != 0 => {
                        *self.port_mut(W_TXSTAT) = 0x0301;
                        self.set_irq(1);
                    }
                    _ => {}
                }
                self.set_status(1);
                self.fire_tx();
                true
            }
        }
    }
}
