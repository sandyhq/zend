//! Play a mono recording on the system's default output device. The stream
//! lives on a thread of its own (a CPAL stream may not leave the thread that
//! opened it) and nothing is written anywhere: the samples are the caller's.
use anyhow::{Context, Result, anyhow, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// One recording playing. Dropping it stops it.
pub struct Playback {
    stop: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
}

impl Playback {
    /// Starts `samples` (mono, `rate` Hz) on the default output device, once
    /// it is open; an error says why it couldn't be (no speaker, a format
    /// it doesn't take).
    pub fn start(samples: Vec<f32>, rate: u32) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(false));
        let (opened, heard) = std::sync::mpsc::sync_channel::<Result<()>>(1);
        let (s, d) = (stop.clone(), done.clone());
        std::thread::Builder::new()
            .name("voice-playback".into())
            .spawn(move || match open(samples, rate, d.clone()) {
                Err(e) => {
                    d.store(true, Ordering::Release);
                    let _ = opened.send(Err(e));
                }
                Ok(stream) => {
                    let _ = opened.send(Ok(()));
                    while !s.load(Ordering::Acquire) && !d.load(Ordering::Acquire) {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    // The device still holds the last buffer it was given.
                    if !s.load(Ordering::Acquire) {
                        std::thread::sleep(std::time::Duration::from_millis(200));
                    }
                    drop(stream);
                    d.store(true, Ordering::Release);
                }
            })?;
        heard
            .recv()
            .map_err(|_| anyhow!("Playback stopped unexpectedly"))??;
        Ok(Self { stop, done })
    }
    /// Whether it has played to its end, failed or been stopped.
    pub fn ended(&self) -> bool {
        self.done.load(Ordering::Acquire)
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        self.stop();
    }
}

fn open(samples: Vec<f32>, rate: u32, done: Arc<AtomicBool>) -> Result<cpal::Stream> {
    let device = cpal::default_host()
        .default_output_device()
        .context("No speaker or headphones available")?;
    let config = device.default_output_config()?;
    let out_rate = config.sample_rate();
    let channels = config.channels().max(1) as usize;
    let samples = crate::resample::convert(samples, rate, out_rate)?;
    let failed = done.clone();
    let err = move |_| failed.store(true, Ordering::Release);
    macro_rules! stream {
        ($sample:ty) => {{
            let mut at = 0usize;
            let done = done.clone();
            device.build_output_stream(
                &config.into(),
                move |out: &mut [$sample], _| {
                    for frame in out.chunks_mut(channels) {
                        let value = match samples.get(at) {
                            Some(s) => <$sample as cpal::Sample>::from_sample(*s),
                            None => {
                                done.store(true, Ordering::Release);
                                <$sample as cpal::Sample>::EQUILIBRIUM
                            }
                        };
                        frame.fill(value);
                        at += 1;
                    }
                },
                err,
                None,
            )?
        }};
    }
    let stream = match config.sample_format() {
        cpal::SampleFormat::I8 => stream!(i8),
        cpal::SampleFormat::I16 => stream!(i16),
        cpal::SampleFormat::I24 => stream!(cpal::I24),
        cpal::SampleFormat::I32 => stream!(i32),
        cpal::SampleFormat::I64 => stream!(i64),
        cpal::SampleFormat::U8 => stream!(u8),
        cpal::SampleFormat::U16 => stream!(u16),
        cpal::SampleFormat::U24 => stream!(cpal::U24),
        cpal::SampleFormat::U32 => stream!(u32),
        cpal::SampleFormat::U64 => stream!(u64),
        cpal::SampleFormat::F32 => stream!(f32),
        cpal::SampleFormat::F64 => stream!(f64),
        _ => bail!("Unsupported speaker format"),
    };
    stream.play()?;
    Ok(stream)
}
