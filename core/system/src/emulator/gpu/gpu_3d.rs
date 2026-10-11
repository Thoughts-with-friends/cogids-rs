//! ARM CPU core
// SPDX-FileCopyrightText: (C) 2017 PSISP
// SPDX-License-Identifier: GPL-3.0-or-later
//! gpu3d.hpp

use crate::Emulator;
use ds_common::interrupt::Interrupt;
use ds_common::memory_map::SCANLINES;
use ds_gpu::engine_3d::gxfifo::CMD_PARAM_AMOUNTS;
use ds_gpu::engine_3d::state::GxCommand;

impl Emulator {
    /// Re-evaluates the GXFIFO interrupt (GXSTAT bits 30-31).
    ///
    /// The interrupt is level triggered (melonDS `GPU3D::CheckFIFOIRQ`):
    /// IF bit 21 follows the condition and is also *cleared* once the FIFO
    /// refills. It must be re-checked whenever the FIFO level changes.
    ///
    /// The Nitro SDK streams display lists with "FIFO less than half full"
    /// interrupts and switches them off from the DMA-complete interrupt of
    /// the last chunk. Leaving bit 21 set after the DMA refilled the FIFO
    /// made it win the IRQ priority over the DMA interrupt forever (an
    /// interrupt storm), which froze 3D scenes such as the HeartGold intro.
    pub fn check_fifo_irq(&mut self) {
        let len = self.gpu.engine_3d.gxfifo.len();
        let raised = match self.gpu.engine_3d.gxfifo_irq_mode() {
            1 => len < 128,
            2 => len == 0,
            _ => false,
        };
        let bit = 1 << (Interrupt::GeometryFifo as u32);
        if raised {
            self.int9_reg.irq_flags |= bit;
        } else {
            self.int9_reg.irq_flags &= !bit;
        }
    }

    pub fn check_gxfifo_irq(&mut self) {
        self.check_fifo_irq();
    }

    pub fn request_fifo_dma(&mut self) {
        if self.gpu.engine_3d.gxfifo.len() < 128 {
            self.gxfifo_dma_request();
        }
    }

    pub fn read_command(&mut self) -> Option<GxCommand> {
        // GXPIPE/GXFIFO are FIFOs (matching C++ `std::queue`), so the command
        // that has been waiting longest must be dequeued from the front.
        // Reading via `front().cloned()` without removing it left the same
        // command in place forever, spinning `gpu3d_run` in an infinite loop
        // on the first GX command sent by a 3D game.
        let cmd = self.gpu.engine_3d.gxpipe.pop_front();

        // Refill the pipe if it is at least half-empty
        let gxpipe_len = self.gpu.engine_3d.gxpipe.len();
        if gxpipe_len < 3 {
            for _ in 0..2 {
                if let Some(front) = self.gpu.engine_3d.gxfifo.pop_front() {
                    self.gpu.engine_3d.gxpipe.push_back(front);
                }
            }

            self.check_fifo_dma();
            self.check_fifo_irq();
        }

        cmd
    }

    fn write_command(&mut self, cmd: GxCommand) {
        #[cfg(feature = "tracing")]
        tracing::trace!(?cmd.command, ?cmd.param);

        if self.gpu.engine_3d.gxfifo.is_empty() && self.gpu.engine_3d.gxpipe.len() < 4 {
            self.gpu.engine_3d.gxpipe.push_back(cmd);
        } else {
            if self.gpu.engine_3d.gxfifo.len() >= 256 {
                while self.gpu.engine_3d.gxfifo.len() >= 256 {
                    self.exec_command();
                }
            }
            self.gpu.engine_3d.gxfifo.push_back(cmd);
            self.check_fifo_irq();
        }
    }

    /// GXSTAT write (`mask` selects the written bytes).
    pub fn set_gxstat(&mut self, word: u32, mask: u32) {
        self.gpu.engine_3d.write_gxstat(word, mask);
        self.check_fifo_irq();
    }

    pub fn write_gxfifo(&mut self, word: u32) {
        if self.gpu.engine_3d.cmd_count == 0 {
            self.gpu.engine_3d.cmd_count = 4;
            self.gpu.engine_3d.current_cmd = word;
            self.gpu.engine_3d.param_count = 0;
            self.gpu.engine_3d.total_params = CMD_PARAM_AMOUNTS[(word & 0xFF) as usize];

            if self.gpu.engine_3d.total_params > 0 {
                return;
            }
        } else {
            self.gpu.engine_3d.param_count += 1;
        }

        loop {
            if (self.gpu.engine_3d.current_cmd & 0xFF) != 0
                || (self.gpu.engine_3d.cmd_count == 4 && self.gpu.engine_3d.current_cmd == 0)
            {
                let cmd = GxCommand {
                    command: (self.gpu.engine_3d.current_cmd & 0xFF) as u8,
                    param: word,
                };
                self.write_command(cmd);
            }

            if self.gpu.engine_3d.param_count >= self.gpu.engine_3d.total_params {
                self.gpu.engine_3d.current_cmd >>= 8;
                self.gpu.engine_3d.cmd_count -= 1;

                if self.gpu.engine_3d.cmd_count == 0 {
                    break;
                }

                self.gpu.engine_3d.param_count = 0;
                self.gpu.engine_3d.total_params =
                    CMD_PARAM_AMOUNTS[(self.gpu.engine_3d.current_cmd & 0xFF) as usize];
            }

            if self.gpu.engine_3d.param_count < self.gpu.engine_3d.total_params {
                break;
            }
        }
    }

    pub fn write_fifo_direct(&mut self, address: u32, word: u32) {
        self.write_command(GxCommand {
            command: ((address >> 2) & 0x7F) as u8,
            param: word,
        });
    }

    pub fn check_fifo_dma(&mut self) {
        if self.gpu.engine_3d.gxfifo.len() < 128 {
            self.gxfifo_dma_request();
        }
    }

    pub fn gpu_handle_event(&mut self) {
        match self.gpu_event.id {
            0 => {
                // Start HBLANK
                #[cfg(feature = "tracing")]
                tracing::trace!("Start HBLANK");

                // If the current line is within the visible screen
                // and the frameskip condition is met, the GPU draws this line.
                if (self.gpu.vertical_count as usize) < SCANLINES
                    && self.gpu.frames_skipped >= self.config.frameskip
                {
                    self.gpu.draw_scanline();
                }

                self.gpu.display_status_arm7.is_hblank = true;
                self.gpu.display_status_arm9.is_hblank = true;

                if self.gpu.display_status_arm7.irq_on_hblank {
                    self.request_interrupt7(Interrupt::HBlank);
                }
                if self.gpu.display_status_arm9.irq_on_hblank {
                    self.request_interrupt9(Interrupt::HBlank);
                }
                if (self.gpu.vertical_count as usize) < SCANLINES {
                    self.hblank_dma_request();
                }

                #[cfg(feature = "tracing")]
                tracing::trace!("Add_gpu_event: 1");

                self.reschedule_gpu_event(1, 99 * 6);
            }
            1 => {
                // End HBLANK
                #[cfg(feature = "tracing")]
                tracing::trace!("End HBLANK");

                self.gpu.display_status_arm7.is_hblank = false;
                self.gpu.display_status_arm9.is_hblank = false;

                self.gpu.display_status_arm7.is_vcounter =
                    self.gpu.vertical_count == self.gpu.display_status_arm7.vcounter;
                if self.gpu.display_status_arm7.is_vcounter
                    && self.gpu.display_status_arm7.irq_on_vcounter
                {
                    self.request_interrupt7(Interrupt::VCountMatch);
                }

                self.gpu.display_status_arm9.is_vcounter =
                    self.gpu.vertical_count == self.gpu.display_status_arm9.vcounter;
                if self.gpu.display_status_arm9.is_vcounter
                    && self.gpu.display_status_arm9.irq_on_vcounter
                {
                    self.request_interrupt9(Interrupt::VCountMatch);
                }

                self.gpu.vertical_count += 1;
                // VBLANK Counter
                #[cfg(feature = "tracing")]
                tracing::trace!("VBLANK Count: {}/{}", self.gpu.vertical_count, SCANLINES);

                // 3D rendering starts at line 215 (melonDS), after the
                // VBlank texture uploads, and is shown in the next frame.
                if self.gpu.vertical_count == 215
                    && self.gpu.frames_skipped >= self.config.frameskip
                {
                    self.gpu.render_3d_frame();
                }

                if self.gpu.vertical_count as usize == SCANLINES {
                    // VBLANK
                    #[cfg(feature = "tracing")]
                    tracing::trace!("Start VBLANK");

                    self.gpu.engine_3d.end_of_frame();
                    self.gpu.frame_complete = true;
                    if self.gpu.display_status_arm7.irq_on_vblank {
                        self.request_interrupt7(Interrupt::VBlank);
                    }
                    if self.gpu.display_status_arm9.irq_on_vblank {
                        self.request_interrupt9(Interrupt::VBlank);
                    }
                    self.gpu.display_status_arm7.is_vblank = true;
                    self.gpu.display_status_arm9.is_vblank = true;

                    self.gpu.engine_upper.vblank_start();
                    self.gpu.engine_lower.vblank_start();
                }
                if self.gpu.vertical_count == 263 {
                    self.gpu.vertical_count = 0;
                    self.gpu.display_status_arm7.is_vblank = false;
                    self.gpu.display_status_arm9.is_vblank = false;

                    match self.gpu.frames_skipped >= self.config.frameskip {
                        true => self.gpu.frames_skipped = 0,
                        false => self.gpu.frames_skipped += 1,
                    }
                }

                self.reschedule_gpu_event(0, 256 * 6);
            }
            unknown_id => {
                #[cfg(feature = "tracing")]
                tracing::error!("Unrecognized event.id: {unknown_id}");
            }
        }
    }
}
