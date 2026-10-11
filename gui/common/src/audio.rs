//! Audio output: plays the SPU's 32768 Hz stereo stream on the default
//! output device (via `cpal`), resampled to the device rate.
use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use cpal::{FromSample, SizedSample};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// Upper bound of queued audio (frames at 32768 Hz, ~150 ms). Older audio
/// is dropped when the emulator runs ahead of the output device.
const MAX_QUEUED_FRAMES: usize = 4900;

#[derive(Default)]
struct Queue {
    frames: VecDeque<[f32; 2]>,
    /// Fractional read position between `frames[0]` and `frames[1]`.
    pos: f64,
    last: [f32; 2],
}

/// Cheap to clone: clones share the same output stream and queue.
#[derive(Clone)]
pub struct AudioOutput {
    queue: Arc<Mutex<Queue>>,
    /// Volume 0.0..=1.0 as `f32` bits.
    volume: Arc<AtomicU32>,
}

impl AudioOutput {
    /// Starts audio output on the default device.
    ///
    /// The `cpal` stream is created and kept on its own thread: on Windows
    /// WASAPI initialises COM in multi-threaded mode, which on the UI thread
    /// conflicts with the window's single-threaded COM setup and made the
    /// window close right after start-up. If no device is usable the
    /// emulator simply runs silently (logged).
    pub fn new(volume: f32) -> Self {
        let queue = Arc::new(Mutex::new(Queue::default()));
        let vol = Arc::new(AtomicU32::new(volume.clamp(0.0, 1.0).to_bits()));

        if std::env::var_os("LUNARIS_NO_AUDIO").is_some() {
            return Self { queue, volume: vol };
        }
        let (q, v) = (Arc::clone(&queue), Arc::clone(&vol));
        let spawned = std::thread::Builder::new()
            .name("audio".to_owned())
            .spawn(move || {
                // Keep the stream alive for the lifetime of the process.
                if let Some(_stream) = open_stream(&q, &v) {
                    loop {
                        std::thread::park();
                    }
                }
            });
        if let Err(e) = spawned {
            tracing::error!("Failed to start audio thread: {e}");
        }

        Self { queue, volume: vol }
    }

    /// Queues interleaved stereo `i16` samples from the emulator.
    pub fn push(&self, samples: &[i16]) {
        let Ok(mut q) = self.queue.lock() else {
            return;
        };
        q.frames.extend(
            samples
                .as_chunks::<2>()
                .0
                .iter()
                .map(|s| [f32::from(s[0]) / 32768.0, f32::from(s[1]) / 32768.0]),
        );
        let excess = q.frames.len().saturating_sub(MAX_QUEUED_FRAMES);
        q.frames.drain(..excess);
    }

    /// Drops queued audio (on stop / pause).
    pub fn clear(&self) {
        if let Ok(mut q) = self.queue.lock() {
            q.frames.clear();
            q.pos = 0.0;
        }
    }

    pub fn set_volume(&self, volume: f32) {
        self.volume
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
}

fn open_stream(queue: &Arc<Mutex<Queue>>, vol: &Arc<AtomicU32>) -> Option<cpal::Stream> {
    let host = cpal::default_host();
    let device = host.default_output_device().or_else(|| {
        tracing::error!("No audio output device");
        None
    })?;
    let config = device
        .default_output_config()
        .inspect_err(|e| tracing::error!("Audio output config: {e}"))
        .ok()?;
    tracing::info!(
        "Audio output: {} ({} Hz, {} ch, {:?})",
        device.name().unwrap_or_default(),
        config.sample_rate().0,
        config.channels(),
        config.sample_format()
    );

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&device, &config.into(), queue, vol),
        cpal::SampleFormat::I16 => build::<i16>(&device, &config.into(), queue, vol),
        cpal::SampleFormat::U16 => build::<u16>(&device, &config.into(), queue, vol),
        other => {
            tracing::error!("Unsupported audio sample format {other:?}");
            return None;
        }
    }?;
    stream
        .play()
        .inspect_err(|e| tracing::error!("Audio play: {e}"))
        .ok()?;
    Some(stream)
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    queue: &Arc<Mutex<Queue>>,
    volume: &Arc<AtomicU32>,
) -> Option<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels);
    let step = f64::from(lunaris_ds_audio::SAMPLE_RATE) / f64::from(config.sample_rate.0);
    let queue = Arc::clone(queue);
    let volume = Arc::clone(volume);

    device
        .build_output_stream(
            config,
            move |out: &mut [T], _| {
                let vol = f32::from_bits(volume.load(Ordering::Relaxed));
                let Ok(mut q) = queue.lock() else {
                    out.fill(T::from_sample(0.0));
                    return;
                };
                for frame in out.chunks_mut(channels) {
                    // Linear interpolation between the two oldest frames;
                    // on underrun hold the last frame (no clicks).
                    let sample = if q.frames.len() >= 2 {
                        let t = q.pos as f32;
                        let (a, b) = (q.frames[0], q.frames[1]);
                        let s = [
                            f32::mul_add(b[0] - a[0], t, a[0]),
                            f32::mul_add(b[1] - a[1], t, a[1]),
                        ];
                        q.pos += step;
                        while q.pos >= 1.0 && q.frames.len() >= 2 {
                            q.pos -= 1.0;
                            q.frames.pop_front();
                        }
                        q.last = s;
                        s
                    } else {
                        q.last
                    };
                    for (i, out) in frame.iter_mut().enumerate() {
                        let v = if channels == 1 {
                            (sample[0] + sample[1]) * 0.5
                        } else {
                            sample[i.min(1)]
                        };
                        *out = T::from_sample(v * vol);
                    }
                }
            },
            |e| tracing::error!("Audio stream error: {e}"),
            None,
        )
        .inspect_err(|e| tracing::error!("Audio stream: {e}"))
        .ok()
}
