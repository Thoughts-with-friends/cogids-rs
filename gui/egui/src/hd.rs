//! Upscaling (internal resolution 2x+, xBRZ) on its own thread, so a heavy
//! upscale does not hold up emulation on the UI thread.
//!
//! ```text
//!  App::draw_screens (new frame)  --Gpu::hd_frame()--try--> worker
//!  worker: HdFrame::screen() x2 -> latest image -> request_repaint
//!  App::draw_screens (next paint) <--take()-- latest image -> textures
//! ```
//!
//! The channel is a rendezvous: a frame is only handed over while the
//! worker is idle; one that arrives while it is busy is not shown (the game
//! keeps its speed). 1x stays on the UI thread (nothing to upscale).
use ds_gpu::display::hires::HdFrame;
use ds_gpu::engine_3d::raster::hires::Renderer;
use eframe::egui;
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, Mutex};

/// Both screens upscaled: `width` pixels wide, 0xAARRGGBB.
pub struct HdImage {
    pub width: usize,
    pub screens: [Vec<u32>; 2],
}

/// The frame, the internal resolution and xBRZ.
type Job = (HdFrame, usize, bool);

pub struct HdWorker {
    jobs: SyncSender<Job>,
    latest: Arc<Mutex<Option<HdImage>>>,
}

impl HdWorker {
    /// Starts the worker; it ends when this is dropped.
    pub fn new(ctx: &egui::Context) -> Self {
        let (jobs, rx) = std::sync::mpsc::sync_channel::<Job>(0);
        let latest = Arc::new(Mutex::new(None));
        let (out, ctx) = (Arc::clone(&latest), ctx.clone());
        let spawned = std::thread::Builder::new()
            .name("hd-render".to_owned())
            .spawn(move || {
                let mut renderer = Renderer::default();
                for (frame, scale, xbrz) in rx {
                    let mut screens = [Vec::new(), Vec::new()];
                    let mut width = 0;
                    for (i, screen) in screens.iter_mut().enumerate() {
                        width = frame.screen(&mut renderer, i == 0, scale, xbrz, screen);
                    }
                    if let Ok(mut latest) = out.lock() {
                        *latest = Some(HdImage { width, screens });
                    }
                    ctx.request_repaint();
                }
            });
        if let Err(e) = spawned {
            log::error!("Failed to start the HD worker: {e}");
        }
        Self { jobs, latest }
    }

    /// Hands `frame` over; `wait` (single steps, settings changes while
    /// paused) waits for a busy worker. False if it was not taken.
    pub fn submit(&self, frame: HdFrame, scale: usize, xbrz: bool, wait: bool) -> bool {
        if wait {
            return self.jobs.send((frame, scale, xbrz)).is_ok();
        }
        match self.jobs.try_send((frame, scale, xbrz)) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => false,
            Err(TrySendError::Disconnected(_)) => {
                log::error!("HD worker stopped");
                false
            }
        }
    }

    /// The image finished since the last call, if any.
    pub fn take(&self) -> Option<HdImage> {
        self.latest.lock().ok()?.take()
    }
}
