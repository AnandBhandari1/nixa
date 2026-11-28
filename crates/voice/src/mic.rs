use crate::aec::{EchoMic, open_echo_mic};
use crate::devices::{device_name, find_device};
use crate::pcm::{Resampler, SAMPLE_RATE, f32_to_i16, to_mono};
use anyhow::{Context, Result, bail};
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

pub(crate) enum MicStream {
    Plain { _stream: cpal::Stream },
    Echo { _mic: Box<EchoMic> },
}

pub(crate) struct MicOpen {
    pub _stream: MicStream,
    pub name: String,
    pub missing: bool,
}

fn build<T>(device: &cpal::Device, config: &StreamConfig, tx: Sender<Vec<i16>>, failed: Arc<AtomicBool>) -> Result<cpal::Stream>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    let mut resampler = Resampler::new(config.sample_rate, SAMPLE_RATE);
    let stream = device.build_input_stream::<T, _, _>(
        *config,
        move |data: &[T], _| {
            let floats: Vec<f32> = data.iter().map(|s| s.to_sample::<f32>()).collect();
            let frame = f32_to_i16(&resampler.push(&to_mono(&floats, channels)));
            if !frame.is_empty() {
                let _ = tx.send(frame);
            }
        },
        move |_| failed.store(true, Ordering::SeqCst),
        None,
    )?;
    Ok(stream)
}

pub(crate) fn open_mic(wanted: Option<&str>, tx: Sender<Vec<i16>>, failed: Arc<AtomicBool>) -> Result<MicOpen> {
    let (device, missing) = find_device(true, wanted)?;
    let name = device_name(&device);
    let supported = device.default_input_config().context("read the mic format")?;
    let config = supported.config();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config, tx, failed)?,
        SampleFormat::I16 => build::<i16>(&device, &config, tx, failed)?,
        SampleFormat::I32 => build::<i32>(&device, &config, tx, failed)?,
        SampleFormat::U16 => build::<u16>(&device, &config, tx, failed)?,
        SampleFormat::F64 => build::<f64>(&device, &config, tx, failed)?,
        other => bail!("mic sample format {other} is not supported"),
    };
    stream.play().context("start the mic")?;
    Ok(MicOpen { _stream: MicStream::Plain { _stream: stream }, name, missing })
}

pub(crate) fn open_echo(wanted: Option<&str>, tx: Sender<Vec<i16>>, failed: Arc<AtomicBool>) -> Result<MicOpen> {
    let mic = open_echo_mic(wanted, true, tx, failed)?;
    Ok(MicOpen { name: mic.name.clone(), missing: mic.missing, _stream: MicStream::Echo { _mic: Box::new(mic) } })
}

pub(crate) fn echo_on(open: &MicOpen) -> bool {
    matches!(&open._stream, MicStream::Echo { _mic } if _mic.echo_cancel)
}
