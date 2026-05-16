use anyhow::{Context, Result, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    Device, FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig,
    SupportedStreamConfig,
};
use rtrb::{Consumer, RingBuffer};
use soundlock_dsp::{Frame, Processor, Profile, Settings};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};
use std::time::Duration;

const RELAXED: Ordering = Ordering::Relaxed;

#[derive(Clone, Debug)]
pub struct Endpoint {
    pub id: String,
    pub name: String,
    pub cable: bool,
}
#[derive(Default)]
pub struct Devices {
    pub outputs: Vec<Endpoint>,
    pub default_output: Option<String>,
}

pub fn is_virtual(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    ["cable", "voicemeeter", "virtual audio"]
        .iter()
        .any(|part| name.contains(part))
}

fn device_name(device: &Device) -> Result<String> {
    let description = device.description()?;
    // WASAPI puts the complete FriendlyName (with driver/model) in extended.
    Ok(description
        .extended()
        .first()
        .cloned()
        .unwrap_or_else(|| description.name().to_owned()))
}

pub fn devices() -> Result<Devices> {
    let host = cpal::default_host();
    let collect = |items: Vec<Device>| -> Vec<Endpoint> {
        items
            .into_iter()
            .filter_map(|d| {
                let name = device_name(&d).ok()?;
                Some(Endpoint {
                    id: d.id().ok()?.to_string(),
                    cable: is_virtual(&name),
                    name,
                })
            })
            .collect()
    };
    Ok(Devices {
        outputs: collect(host.output_devices()?.collect()),
        default_output: host
            .default_output_device()
            .and_then(|d| d.id().ok())
            .map(|id| id.to_string()),
    })
}

/// Only atomic values cross the real-time/UI boundary; no locks in callbacks.
pub struct Shared {
    boost: AtomicU32,
    profile: AtomicU32,
    enabled: AtomicBool,
    pub input_peak: AtomicU32,
    pub output_peak: AtomicU32,
    pub reduction: AtomicU32,
    pub underruns: AtomicU64,
    pub overruns: AtomicU64,
    pub failed: AtomicBool,
    pub peak_limit: AtomicU32,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            boost: AtomicU32::new(1.0_f32.to_bits()),
            profile: AtomicU32::new(0),
            enabled: AtomicBool::new(true),
            input_peak: AtomicU32::new(0),
            output_peak: AtomicU32::new(0),
            reduction: AtomicU32::new(0),
            underruns: AtomicU64::new(0),
            overruns: AtomicU64::new(0),
            failed: AtomicBool::new(false),
            peak_limit: AtomicU32::new(0.5_f32.to_bits()),
        }
    }
}

impl Shared {
    pub fn update(&self, settings: Settings) {
        self.boost.store(settings.boost.to_bits(), RELAXED);
        self.profile.store(
            match settings.profile {
                Profile::Night => 0,
                Profile::Headphones => 1,
                Profile::Speakers => 2,
            },
            RELAXED,
        );
        self.enabled.store(settings.enabled, RELAXED);
    }
    pub fn settings(&self) -> Settings {
        Settings {
            boost: f32::from_bits(self.boost.load(RELAXED)),
            enabled: self.enabled.load(RELAXED),
            profile: match self.profile.load(RELAXED) {
                1 => Profile::Headphones,
                2 => Profile::Speakers,
                _ => Profile::Night,
            },
        }
    }
    pub fn meter(value: &AtomicU32) -> f32 {
        f32::from_bits(value.load(RELAXED))
    }
}

pub struct Engine {
    // Drop capture before playback; neither stream outlives the engine.
    _input: Stream,
    _output: Stream,
    pub description: String,
}

fn find_device(id: &str, input: bool) -> Result<Device> {
    let host = cpal::default_host();
    let device = if input {
        host.input_devices()?
            .find(|d| d.id().is_ok_and(|v| v.to_string() == id))
    } else {
        host.output_devices()?
            .find(|d| d.id().is_ok_and(|v| v.to_string() == id))
    };
    device.context("The device is disconnected. Refresh devices and select it again.")
}

fn supported_format(format: SampleFormat) -> bool {
    matches!(
        format,
        SampleFormat::F32
            | SampleFormat::I16
            | SampleFormat::U16
            | SampleFormat::I32
            | SampleFormat::F64
    )
}

fn choose_config(device: &Device, input: bool) -> Result<SupportedStreamConfig> {
    let default = if input {
        device.default_input_config()
    } else {
        device.default_output_config()
    };
    if let Ok(config) = default
        && (1..=2).contains(&config.channels())
        && supported_format(config.sample_format())
    {
        return Ok(config);
    }
    let ranges: Vec<_> = if input {
        device.supported_input_configs()?.collect()
    } else {
        device.supported_output_configs()?.collect()
    };
    ranges
        .into_iter()
        .filter(|r| (1..=2).contains(&r.channels()) && supported_format(r.sample_format()))
        .max_by_key(|r| (r.channels() == 2, r.sample_format() == SampleFormat::F32))
        .map(|r| {
            let rate = 48_000_u32.clamp(r.min_sample_rate(), r.max_sample_rate());
            r.with_sample_rate(rate)
        })
        .context("A mono or stereo device is required. Select stereo in Windows sound settings.")
}

impl Engine {
    pub fn start(input_id: &str, output_id: &str, shared: Arc<Shared>) -> Result<Self> {
        let input = find_device(input_id, false)?;
        let output = find_device(output_id, false)?;
        let output_name = device_name(&output)?;
        if input_id == output_id {
            bail!("Source and output must be different devices to prevent feedback.");
        }
        if is_virtual(&output_name) {
            bail!("Select physical headphones or speakers as the output.");
        }
        // On Windows CPAL supports loopback capture on a rendering endpoint.
        // Query its OUTPUT format, then create an INPUT stream on the same device.
        let in_supported = choose_config(&input, false)?;
        let out_supported = choose_config(&output, false)?;
        let in_rate = in_supported.sample_rate();
        let out_rate = out_supported.sample_rate();
        let in_config: StreamConfig = in_supported.clone().into();
        let out_config: StreamConfig = out_supported.clone().into();
        let (producer, consumer) = RingBuffer::<Frame>::new((in_rate as usize / 4).max(4096));
        shared.failed.store(false, RELAXED);
        shared.underruns.store(0, RELAXED);
        shared.overruns.store(0, RELAXED);
        macro_rules! capture {
            ($t:ty) => {
                capture::<$t>(&input, &in_config, producer, shared.clone())?
            };
        }
        let input_stream = match in_supported.sample_format() {
            SampleFormat::F32 => capture!(f32),
            SampleFormat::I16 => capture!(i16),
            SampleFormat::U16 => capture!(u16),
            SampleFormat::I32 => capture!(i32),
            SampleFormat::F64 => capture!(f64),
            _ => bail!("Unsupported input sample format"),
        };
        let playback = Playback::new(consumer, in_rate, out_rate);
        macro_rules! render {
            ($t:ty) => {
                render::<$t>(&output, &out_config, playback, shared.clone())?
            };
        }
        let output_stream = match out_supported.sample_format() {
            SampleFormat::F32 => render!(f32),
            SampleFormat::I16 => render!(i16),
            SampleFormat::U16 => render!(u16),
            SampleFormat::I32 => render!(i32),
            SampleFormat::F64 => render!(f64),
            _ => bail!("Unsupported output sample format"),
        };
        input_stream.play().context("Could not start capture")?;
        output_stream.play().context("Could not start playback")?;
        Ok(Self {
            _input: input_stream,
            _output: output_stream,
            description: format!(
                "{} / {} Hz · ~40 ms buffer + 5 ms limiter",
                in_rate, out_rate
            ),
        })
    }
}

fn capture<T: SizedSample>(
    device: &Device,
    config: &StreamConfig,
    mut producer: rtrb::Producer<Frame>,
    shared: Arc<Shared>,
) -> Result<Stream>
where
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    let error_shared = shared.clone();
    Ok(device.build_input_stream(
        config,
        move |data: &[T], _| {
            let mut dropped = 0;
            for frame in data.chunks_exact(channels) {
                let left = f32::from_sample(frame[0]);
                let right = if channels > 1 {
                    f32::from_sample(frame[1])
                } else {
                    left
                };
                if producer.push([left, right]).is_err() {
                    dropped += 1;
                }
            }
            if dropped > 0 {
                shared.overruns.fetch_add(dropped, RELAXED);
            }
        },
        move |_| {
            error_shared.failed.store(true, RELAXED);
        },
        Some(Duration::from_secs(3)),
    )?)
}

/// Linear asynchronous resampling with slow occupancy feedback for independent clocks.
/// This MVP favors bounded latency; a band-limited resampler is a future quality upgrade.
struct Playback {
    consumer: Consumer<Frame>,
    previous: Frame,
    next: Frame,
    phase: f64,
    nominal_ratio: f64,
    ratio: f64,
    target: usize,
    ready: bool,
    fade: f32,
    fade_step: f32,
    processor: Processor,
}

impl Playback {
    fn new(consumer: Consumer<Frame>, input_rate: u32, output_rate: u32) -> Self {
        let ratio = input_rate as f64 / output_rate as f64;
        Self {
            consumer,
            previous: [0.0; 2],
            next: [0.0; 2],
            phase: 0.0,
            nominal_ratio: ratio,
            ratio,
            target: (input_rate as usize * 40 / 1000).max(2),
            ready: false,
            fade: 0.0,
            fade_step: 1.0 / (output_rate as f32 * 0.005),
            processor: Processor::new(output_rate),
        }
    }
    fn begin_block(&mut self) {
        let available = self.consumer.slots();
        if available > self.target * 3 {
            for _ in 0..available - self.target {
                let _ = self.consumer.pop();
            }
            self.ready = false;
        }
        let error = (self.consumer.slots() as f64 - self.target as f64) / self.target as f64;
        let wanted = self.nominal_ratio * (1.0 + (error * 0.002).clamp(-0.005, 0.005));
        self.ratio += (wanted - self.ratio) * 0.01;
    }
    fn next_frame(&mut self) -> Option<Frame> {
        if !self.ready {
            if self.consumer.slots() < self.target {
                return None;
            }
            self.previous = self.consumer.pop().ok()?;
            self.next = self.consumer.pop().ok()?;
            self.phase = 0.0;
            self.fade = 0.0;
            self.ready = true;
        }
        while self.phase >= 1.0 {
            self.previous = self.next;
            match self.consumer.pop() {
                Ok(frame) => self.next = frame,
                Err(_) => {
                    self.ready = false;
                    return None;
                }
            }
            self.phase -= 1.0;
        }
        let t = self.phase as f32;
        let frame =
            std::array::from_fn(|ch| self.previous[ch] + (self.next[ch] - self.previous[ch]) * t);
        self.phase += self.ratio;
        self.fade = (self.fade + self.fade_step).min(1.0);
        Some(frame.map(|v| v * self.fade))
    }
}

fn render<T: SizedSample + FromSample<f32>>(
    device: &Device,
    config: &StreamConfig,
    mut playback: Playback,
    shared: Arc<Shared>,
) -> Result<Stream> {
    let channels = config.channels as usize;
    let error_shared = shared.clone();
    Ok(device.build_output_stream(
        config,
        move |data: &mut [T], _| {
            playback.begin_block();
            let settings = shared.settings();
            let mut input_peak = 0.0_f32;
            let mut output_peak = 0.0_f32;
            let mut reduction = 0.0_f32;
            let mut missing = 0;
            for frame in data.chunks_exact_mut(channels) {
                let input = match playback.next_frame() {
                    Some(frame) => frame,
                    None => {
                        missing += 1;
                        [0.0; 2]
                    }
                };
                let out = playback.processor.process(input, settings);
                let meters = playback.processor.meters();
                input_peak = input_peak.max(meters.input_peak);
                output_peak = output_peak.max(meters.output_peak);
                reduction = reduction.max(meters.reduction_db);
                frame[0] = T::from_sample(if channels == 1 {
                    (out[0] + out[1]) * 0.5
                } else {
                    out[0]
                });
                if channels > 1 {
                    frame[1] = T::from_sample(out[1]);
                }
            }
            shared.input_peak.store(input_peak.to_bits(), RELAXED);
            shared.output_peak.store(output_peak.to_bits(), RELAXED);
            shared.reduction.store(reduction.to_bits(), RELAXED);
            if missing > 0 {
                shared.underruns.fetch_add(missing, RELAXED);
            }
        },
        move |_| {
            error_shared.failed.store(true, RELAXED);
        },
        Some(Duration::from_secs(3)),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routes_never_classify_normal_speakers_as_cable() {
        assert!(is_virtual("CABLE Output (VB-Audio Virtual Cable)"));
        assert!(is_virtual("Voicemeeter Output"));
        assert!(!is_virtual("Headphones (Realtek Audio)"));
    }
    #[test]
    fn empty_capture_produces_silence_without_blocking() {
        let (_tx, rx) = RingBuffer::new(4096);
        let mut playback = Playback::new(rx, 48_000, 48_000);
        for _ in 0..1000 {
            assert!(playback.next_frame().is_none());
        }
    }
    #[test]
    fn differing_rates_preserve_duration_and_stereo() {
        for (input, output) in [(44_100, 48_000), (48_000, 44_100), (96_000, 48_000)] {
            let (mut tx, rx) = RingBuffer::new(input as usize + 1);
            for i in 0..input {
                tx.push([i as f32 / input as f32, -(i as f32 / input as f32)])
                    .unwrap();
            }
            let mut p = Playback::new(rx, input, output);
            let mut count = 0;
            while let Some(frame) = p.next_frame() {
                assert!((frame[0] + frame[1]).abs() < 1e-6);
                count += 1;
            }
            assert!(
                (count as i64 - output as i64).abs() < 4,
                "{input} → {output}: {count}"
            );
        }
    }
    #[test]
    fn large_backlog_is_trimmed_to_keep_latency_bounded() {
        let (mut tx, rx) = RingBuffer::new(10_000);
        for _ in 0..9000 {
            tx.push([0.1; 2]).unwrap();
        }
        let mut p = Playback::new(rx, 48_000, 48_000);
        p.begin_block();
        assert!(p.consumer.slots() <= p.target);
        assert!(p.next_frame().is_some());
    }
    #[test]
    fn stream_refills_after_underrun() {
        let (mut tx, rx) = RingBuffer::new(4096);
        let mut p = Playback::new(rx, 48_000, 48_000);
        for _ in 0..2000 {
            tx.push([0.2; 2]).unwrap();
        }
        for _ in 0..3000 {
            let _ = p.next_frame();
        }
        assert!(!p.ready);
        for _ in 0..2000 {
            tx.push([0.2; 2]).unwrap();
        }
        assert!(p.next_frame().is_some());
        assert!(p.fade < 0.01);
    }
}
