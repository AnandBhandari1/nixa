use crate::devices::pick_name;
use crate::pcm::{Resampler, SAMPLE_RATE, f32_to_i16};
use anyhow::{Result, anyhow, bail};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::{Encoding, RefEncode, msg_send};
use objc2::runtime::{AnyObject, Bool, ProtocolObject};
use objc2_audio_toolbox::{AudioUnitGetProperty, AudioUnitSetProperty, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global};
use objc2_avf_audio::{
    AVAudioEngine, AVAudioEngineConfigurationChangeNotification, AVAudioInputNode, AVAudioPCMBuffer, AVAudioTime,
    AVAudioVoiceProcessingOtherAudioDuckingConfiguration, AVAudioVoiceProcessingOtherAudioDuckingLevel,
};
use objc2_core_audio::{
    AudioObjectGetPropertyData, AudioObjectGetPropertyDataSize, AudioObjectID, AudioObjectPropertyAddress,
    kAudioDevicePropertyScopeInput, kAudioDevicePropertyStreams, kAudioHardwarePropertyDefaultInputDevice,
    kAudioHardwarePropertyDevices, kAudioObjectPropertyElementMain, kAudioObjectPropertyName,
    kAudioObjectPropertyScopeGlobal, kAudioObjectSystemObject,
};
use objc2_core_foundation::{CFRetained, CFRunLoop, CFRunLoopRunResult, CFString, kCFRunLoopDefaultMode};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectProtocol};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Sender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const TAP_FRAMES: u32 = 4096;
const LOOP_SECONDS: f64 = 0.1;
const START_WAIT: Duration = Duration::from_secs(10);

#[repr(C)]
struct UnitRecord {
    _data: [i64; 1],
}

unsafe impl RefEncode for UnitRecord {
    const ENCODING_REF: Encoding =
        Encoding::Pointer(&Encoding::Struct("ComponentInstanceRecord", &[Encoding::Array(1, &Encoding::LongLong)]));
}

static ECHO_OPEN: AtomicUsize = AtomicUsize::new(0);

type TapBlock = RcBlock<dyn Fn(NonNull<AVAudioPCMBuffer>, NonNull<AVAudioTime>)>;
type ChangeBlock = RcBlock<dyn Fn(NonNull<NSNotification>)>;

pub struct EchoMic {
    pub name: String,
    pub missing: bool,
    pub echo_cancel: bool,
    pub rate: u32,
    pub channels: u32,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for EchoMic {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Info {
    name: String,
    missing: bool,
    echo_cancel: bool,
    rate: u32,
    channels: u32,
}

struct Engine {
    engine: Retained<AVAudioEngine>,
    input: Retained<AVAudioInputNode>,
    observer: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    echo: bool,
    _tap: TapBlock,
    _on_change: ChangeBlock,
}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe {
            self.engine.stop();
            self.input.removeTapOnBus(0);
            let observer: &AnyObject = self.observer.as_ref();
            NSNotificationCenter::defaultCenter().removeObserver(observer);
        }
        if self.echo {
            ECHO_OPEN.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

pub fn echo_cancelled() -> bool {
    ECHO_OPEN.load(Ordering::SeqCst) > 0
}

pub fn use_echo_cancel(wanted: bool, headphones: bool) -> bool {
    wanted && !headphones
}

pub fn mic_open_during_voice(headphones: bool, echo_cancelled: bool, phone: bool) -> bool {
    !phone && (headphones || echo_cancelled)
}

fn address(selector: u32, scope: u32) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress { mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain }
}

fn data_size(id: AudioObjectID, addr: &AudioObjectPropertyAddress) -> Option<u32> {
    let mut size = 0u32;
    let status = unsafe {
        AudioObjectGetPropertyDataSize(id, NonNull::from(addr), 0, std::ptr::null(), NonNull::from(&mut size))
    };
    (status == 0).then_some(size)
}

fn device_ids() -> Vec<AudioObjectID> {
    let addr = address(kAudioHardwarePropertyDevices, kAudioObjectPropertyScopeGlobal);
    let system = kAudioObjectSystemObject as AudioObjectID;
    let Some(mut size) = data_size(system, &addr) else {
        return Vec::new();
    };
    let mut ids = vec![0 as AudioObjectID; size as usize / size_of::<AudioObjectID>()];
    if ids.is_empty() {
        return ids;
    }
    let status = unsafe {
        AudioObjectGetPropertyData(
            system,
            NonNull::from(&addr),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::new_unchecked(ids.as_mut_ptr().cast::<c_void>()),
        )
    };
    if status != 0 {
        return Vec::new();
    }
    ids.truncate(size as usize / size_of::<AudioObjectID>());
    ids
}

fn has_input(id: AudioObjectID) -> bool {
    data_size(id, &address(kAudioDevicePropertyStreams, kAudioDevicePropertyScopeInput)).is_some_and(|s| s > 0)
}

fn device_name(id: AudioObjectID) -> Option<String> {
    let addr = address(kAudioObjectPropertyName, kAudioObjectPropertyScopeGlobal);
    let mut name: *const CFString = std::ptr::null();
    let mut size = size_of::<*const CFString>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            id,
            NonNull::from(&addr),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut name).cast::<c_void>(),
        )
    };
    let name = NonNull::new(name.cast_mut()).filter(|_| status == 0)?;
    let name = unsafe { CFRetained::from_raw(name) };
    Some(name.to_string())
}

fn default_input() -> Option<AudioObjectID> {
    let addr = address(kAudioHardwarePropertyDefaultInputDevice, kAudioObjectPropertyScopeGlobal);
    let mut id: AudioObjectID = 0;
    let mut size = size_of::<AudioObjectID>() as u32;
    let status = unsafe {
        AudioObjectGetPropertyData(
            kAudioObjectSystemObject as AudioObjectID,
            NonNull::from(&addr),
            0,
            std::ptr::null(),
            NonNull::from(&mut size),
            NonNull::from(&mut id).cast::<c_void>(),
        )
    };
    (status == 0 && id != 0).then_some(id)
}

pub(crate) fn input_devices() -> Vec<(AudioObjectID, String)> {
    device_ids()
        .into_iter()
        .filter(|id| has_input(*id))
        .filter_map(|id| device_name(id).map(|n| (id, n)))
        .collect()
}

fn choose(wanted: Option<&str>) -> (Option<(AudioObjectID, String)>, bool) {
    let Some(wanted) = wanted.map(str::trim).filter(|w| !w.is_empty()) else {
        return (None, false);
    };
    let devices = input_devices();
    let names: Vec<String> = devices.iter().map(|(_, n)| n.clone()).collect();
    match pick_name(&names, wanted) {
        Some(i) => (Some(devices[i].clone()), false),
        None => (None, true),
    }
}

fn audio_unit(input: &AVAudioInputNode) -> *mut UnitRecord {
    unsafe { msg_send![input, audioUnit] }
}

fn unit_device(input: &AVAudioInputNode) -> Option<AudioObjectID> {
    let unit = audio_unit(input);
    if unit.is_null() {
        return None;
    }
    let mut id: AudioObjectID = 0;
    let mut size = size_of::<AudioObjectID>() as u32;
    let status = unsafe {
        AudioUnitGetProperty(
            unit.cast(),
            kAudioOutputUnitProperty_CurrentDevice,
            kAudioUnitScope_Global,
            0,
            NonNull::from(&mut id).cast::<c_void>(),
            NonNull::from(&mut size),
        )
    };
    (status == 0 && id != 0).then_some(id)
}

fn set_device(input: &AVAudioInputNode, id: AudioObjectID) -> Result<()> {
    let unit = audio_unit(input);
    if unit.is_null() {
        bail!("the mic has no audio unit");
    }
    let status = unsafe {
        AudioUnitSetProperty(
            unit.cast(),
            kAudioOutputUnitProperty_CurrentDevice,
            kAudioUnitScope_Global,
            0,
            (&id as *const AudioObjectID).cast::<c_void>(),
            size_of::<AudioObjectID>() as u32,
        )
    };
    if status != 0 {
        bail!("could not pick the mic (status {status})");
    }
    Ok(())
}

fn first_channel(buffer: &AVAudioPCMBuffer) -> Vec<f32> {
    unsafe {
        let frames = buffer.frameLength() as usize;
        let data = buffer.floatChannelData();
        if frames == 0 || data.is_null() {
            return Vec::new();
        }
        let stride = buffer.stride().max(1);
        let first = (*data).as_ptr();
        (0..frames).map(|i| *first.add(i * stride)).collect()
    }
}

fn tap_block(rate: u32, tx: Sender<Vec<i16>>) -> TapBlock {
    let resampler = Mutex::new(Resampler::new(rate, SAMPLE_RATE));
    RcBlock::new(move |buffer: NonNull<AVAudioPCMBuffer>, _: NonNull<AVAudioTime>| {
        let samples = first_channel(unsafe { buffer.as_ref() });
        if samples.is_empty() {
            return;
        }
        let pcm = resampler.lock().map(|mut r| f32_to_i16(&r.push(&samples))).unwrap_or_default();
        if !pcm.is_empty() {
            let _ = tx.send(pcm);
        }
    })
}

fn start_engine(wanted: Option<&str>, echo_cancel: bool, tx: Sender<Vec<i16>>, failed: Arc<AtomicBool>) -> Result<(Engine, Info)> {
    let (chosen, missing) = choose(wanted);
    let engine = unsafe { AVAudioEngine::new() };
    let input = unsafe { engine.inputNode() };
    if let Some((id, _)) = &chosen {
        set_device(&input, *id)?;
    }
    if echo_cancel {
        unsafe {
            input
                .setVoiceProcessingEnabled_error(true)
                .map_err(|e| anyhow!("echo cancel not available: {}", e.localizedDescription()))?;
            input.setVoiceProcessingOtherAudioDuckingConfiguration(AVAudioVoiceProcessingOtherAudioDuckingConfiguration {
                enableAdvancedDucking: Bool::NO,
                duckingLevel: AVAudioVoiceProcessingOtherAudioDuckingLevel::Min,
            });
        }
        if let Some((id, _)) = &chosen
            && unit_device(&input) != Some(*id)
        {
            set_device(&input, *id)?;
        }
    }
    let name = unit_device(&input)
        .or_else(default_input)
        .and_then(device_name)
        .unwrap_or_else(|| "default".into());
    let format = unsafe { input.outputFormatForBus(0) };
    let (rate, channels) = unsafe { (format.sampleRate(), format.channelCount()) };
    if channels == 0 || rate <= 0.0 {
        bail!("the mic has no input format");
    }
    let rate = rate.round() as u32;
    let tap = tap_block(rate, tx);
    unsafe { input.installTapOnBus_bufferSize_format_block(0, TAP_FRAMES, Some(&format), RcBlock::as_ptr(&tap)) };
    let watched = engine.clone();
    let on_change: ChangeBlock = RcBlock::new(move |_: NonNull<NSNotification>| {
        if !unsafe { watched.isRunning() } {
            failed.store(true, Ordering::SeqCst);
        }
    });
    let observer = unsafe {
        let engine_obj: &AnyObject = engine.as_ref();
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(AVAudioEngineConfigurationChangeNotification),
            Some(engine_obj),
            None,
            &on_change,
        )
    };
    let echo = unsafe { input.isVoiceProcessingEnabled() };
    if echo {
        ECHO_OPEN.fetch_add(1, Ordering::SeqCst);
    }
    let held = Engine { engine, input, observer, echo, _tap: tap, _on_change: on_change };
    unsafe { held.engine.prepare() };
    if let Err(e) = unsafe { held.engine.startAndReturnError() } {
        bail!("mic did not start: {}", e.localizedDescription());
    }
    if echo_cancel && !echo {
        bail!("echo cancel not available: voice processing is off after start");
    }
    Ok((held, Info { name, missing, echo_cancel: echo, rate, channels }))
}

fn run_loop(stop: &AtomicBool) {
    while !stop.load(Ordering::SeqCst) {
        let result = CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, LOOP_SECONDS, false);
        if result == CFRunLoopRunResult::Finished {
            thread::sleep(Duration::from_secs_f64(LOOP_SECONDS));
        }
    }
}

pub fn open_echo_mic(wanted: Option<&str>, echo_cancel: bool, tx: Sender<Vec<i16>>, failed: Arc<AtomicBool>) -> Result<EchoMic> {
    let wanted = wanted.map(str::to_string);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_here = stop.clone();
    let (ready_tx, ready_rx) = sync_channel::<Result<Info>>(1);
    let thread = thread::Builder::new().name("nixa-echo-mic".into()).spawn(move || {
        match start_engine(wanted.as_deref(), echo_cancel, tx, failed) {
            Ok((engine, info)) => {
                let _ = ready_tx.send(Ok(info));
                run_loop(&stop_here);
                drop(engine);
            }
            Err(e) => {
                let _ = ready_tx.send(Err(e));
            }
        }
    })?;
    let mut mic = EchoMic { name: String::new(), missing: false, echo_cancel: false, rate: 0, channels: 0, stop, thread: Some(thread) };
    match ready_rx.recv_timeout(START_WAIT) {
        Ok(Ok(info)) => {
            mic.name = info.name;
            mic.missing = info.missing;
            mic.echo_cancel = info.echo_cancel;
            mic.rate = info.rate;
            mic.channels = info.channels;
            Ok(mic)
        }
        Ok(Err(e)) => Err(e),
        Err(_) => bail!("the mic did not start within {} s", START_WAIT.as_secs()),
    }
}

pub fn ready_line(mic: &EchoMic) -> String {
    format!(
        "ready echoCancel={} device={} rate={} channels={}{}",
        u8::from(mic.echo_cancel),
        mic.name,
        mic.rate,
        mic.channels,
        if mic.missing { " missing=1" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;
    use std::time::Instant;

    #[test]
    fn headphones_turn_echo_cancel_off() {
        assert!(use_echo_cancel(true, false));
        assert!(!use_echo_cancel(true, true));
        assert!(!use_echo_cancel(false, false));
    }

    #[test]
    fn mic_stays_open_while_speaking_only_when_safe() {
        assert!(mic_open_during_voice(true, false, false));
        assert!(mic_open_during_voice(false, true, false));
        assert!(!mic_open_during_voice(false, false, false));
        assert!(!mic_open_during_voice(true, true, true));
    }

    #[test]
    fn unknown_mic_name_is_missing() {
        let (chosen, missing) = choose(Some("no such mic 7f3a"));
        assert!(chosen.is_none() && missing);
        assert_eq!(choose(Some("  ")), (None, false));
        assert_eq!(choose(None), (None, false));
    }

    #[test]
    fn input_devices_have_names() {
        for (id, name) in input_devices() {
            assert!(id != 0 && !name.trim().is_empty(), "{id} {name:?}");
        }
    }

    static MIC_LOCK: Mutex<()> = Mutex::new(());

    fn hear(device: Option<&str>, echo: bool, ms: u64) -> (EchoMic, usize) {
        let _guard = MIC_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (tx, rx) = channel();
        let mic = open_echo_mic(device, echo, tx, Arc::new(AtomicBool::new(false))).unwrap();
        let until = Instant::now() + Duration::from_millis(ms);
        let mut samples = 0;
        while Instant::now() < until {
            if let Ok(frame) = rx.recv_timeout(Duration::from_millis(50)) {
                samples += frame.len();
            }
        }
        (mic, samples)
    }

    #[test]
    #[ignore]
    fn echo_mic_gives_16k_audio() {
        let device = std::env::var("NIXA_TEST_MIC").ok();
        let (mic, samples) = hear(device.as_deref(), true, 2000);
        eprintln!("{} samples={samples}", ready_line(&mic));
        assert!(mic.echo_cancel);
        assert!(echo_cancelled());
        assert!(samples > SAMPLE_RATE as usize / 2, "{samples}");
        drop(mic);
        assert!(!echo_cancelled());
    }

    #[test]
    #[ignore]
    fn plain_mic_without_echo_cancel() {
        let device = std::env::var("NIXA_TEST_MIC").ok();
        let (mic, samples) = hear(device.as_deref(), false, 1000);
        eprintln!("{} samples={samples}", ready_line(&mic));
        assert!(!mic.echo_cancel);
        assert!(samples > SAMPLE_RATE as usize / 3, "{samples}");
    }
}
