//! Driver-free reactive peak reduction through Windows Core Audio.
//! This controls endpoint gain; it is not a PCM look-ahead limiter or a booster.
use crate::audio::Shared;
use soundlock_dsp::{Profile, db_to_gain, gain_to_db};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;

pub struct Guard {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Guard {
    pub fn start(device_id: &str, shared: Arc<Shared>) -> anyhow::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let id = device_id.to_owned();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        shared.failed.store(false, Ordering::Relaxed);
        let worker = std::thread::spawn(move || {
            match platform::Control::open(&id) {
                Ok(mut control) => {
                    let _ = sender.send(Ok(()));
                    while !worker_stop.load(Ordering::Relaxed) {
                        if control.tick(&shared).is_err() {
                            shared.failed.store(true, Ordering::Relaxed);
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    // Control restores the last user-selected volume on drop.
                }
                Err(e) => {
                    let _ = sender.send(Err(format!("{e:#}")));
                }
            }
        });
        match receiver.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Self {
                stop,
                worker: Some(worker),
            }),
            result => {
                stop.store(true, Ordering::Relaxed);
                let message = match result {
                    Ok(Err(e)) => e,
                    _ => "Core Audio did not respond".into(),
                };
                // A delayed COM activation must not block the UI indefinitely.
                anyhow::bail!(message)
            }
        }
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn target_reduction(peak: f32, limit: f32, enabled: bool) -> f32 {
    if !enabled || !peak.is_finite() || peak <= 0.0 {
        return 0.0;
    }
    gain_to_db((limit.clamp(0.1, 1.0) / peak).min(1.0)).max(-36.0)
}

#[cfg(windows)]
mod platform {
    use super::*;
    use windows::{
        Win32::{
            Media::Audio::{
                Endpoints::{IAudioEndpointVolume, IAudioMeterInformation},
                IMMDeviceEnumerator, MMDeviceEnumerator,
            },
            System::Com::{
                CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
            },
        },
        core::PCWSTR,
    };

    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            unsafe {
                CoUninitialize();
            }
        }
    }

    pub struct Control {
        volume: IAudioEndpointVolume,
        meter: IAudioMeterInformation,
        baseline: f32,
        last_set: f32,
        minimum: f32,
        gain: f32,
        _com: Com,
    }
    impl Control {
        pub fn open(id: &str) -> anyhow::Result<Self> {
            // SAFETY: all COM objects are created, used and released on this
            // worker thread, before its matching CoUninitialize.
            unsafe {
                CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
                let com = Com;
                let enumerator: IMMDeviceEnumerator =
                    CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
                let id: Vec<u16> = id
                    .strip_prefix("wasapi:")
                    .unwrap_or(id)
                    .encode_utf16()
                    .chain(Some(0))
                    .collect();
                let device = enumerator.GetDevice(PCWSTR(id.as_ptr()))?;
                let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
                let meter: IAudioMeterInformation = device.Activate(CLSCTX_ALL, None)?;
                let baseline = volume.GetMasterVolumeLevel()?;
                let (mut minimum, mut maximum, mut step) = (0.0, 0.0, 0.0);
                volume.GetVolumeRange(&mut minimum, &mut maximum, &mut step)?;
                Ok(Self {
                    volume,
                    meter,
                    baseline,
                    last_set: baseline,
                    minimum,
                    gain: 0.0,
                    _com: com,
                })
            }
        }
        pub fn tick(&mut self, shared: &Shared) -> anyhow::Result<()> {
            unsafe {
                let current = self.volume.GetMasterVolumeLevel()?;
                if (current - self.last_set).abs() > 0.25 {
                    // Respect keyboard/mixer changes, including when the user
                    // lowers volume during a loud scene. Never restore above it.
                    self.baseline = current;
                    self.last_set = current;
                    self.gain = 0.0;
                }
                let peak = self.meter.GetPeakValue()?;
                let settings = shared.settings();
                let wanted =
                    target_reduction(peak, Shared::meter(&shared.peak_limit), settings.enabled);
                let release_ms = match settings.profile {
                    Profile::Night => 1100.0,
                    Profile::Headphones => 700.0,
                    Profile::Speakers => 450.0,
                };
                let coeff = if wanted < self.gain {
                    0.15
                } else {
                    (-10.0_f32 / release_ms).exp()
                };
                self.gain = coeff * self.gain + (1.0 - coeff) * wanted;
                let level = (self.baseline + self.gain)
                    .max(self.minimum)
                    .min(self.baseline);
                if (level - self.last_set).abs() > 0.05 {
                    self.volume.SetMasterVolumeLevel(level, std::ptr::null())?;
                    // Store the actual (possibly quantized) driver value.
                    self.last_set = self.volume.GetMasterVolumeLevel()?;
                }
                shared.input_peak.store(peak.to_bits(), Ordering::Relaxed);
                shared
                    .output_peak
                    .store((peak * db_to_gain(self.gain)).to_bits(), Ordering::Relaxed);
                shared
                    .reduction
                    .store((-self.gain).to_bits(), Ordering::Relaxed);
            }
            Ok(())
        }
        pub fn probe(&self) -> anyhow::Result<(f32, f32)> {
            unsafe {
                Ok((
                    self.volume.GetMasterVolumeLevel()?,
                    self.meter.GetPeakValue()?,
                ))
            }
        }
    }
    impl Drop for Control {
        fn drop(&mut self) {
            unsafe {
                if let Ok(current) = self.volume.GetMasterVolumeLevel()
                    && (current - self.last_set).abs() <= 0.25
                    && (current - self.baseline).abs() > 0.05
                {
                    let _ = self
                        .volume
                        .SetMasterVolumeLevel(self.baseline, std::ptr::null());
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    pub struct Control;
    impl Control {
        pub fn open(_: &str) -> anyhow::Result<Self> {
            anyhow::bail!("System protection requires Windows")
        }
        pub fn tick(&mut self, _: &Shared) -> anyhow::Result<()> {
            Ok(())
        }
        pub fn probe(&self) -> anyhow::Result<(f32, f32)> {
            anyhow::bail!("Windows only")
        }
    }
}

pub fn probe(id: &str) -> anyhow::Result<(f32, f32)> {
    let id = id.to_owned();
    std::thread::spawn(move || platform::Control::open(&id)?.probe())
        .join()
        .map_err(|_| anyhow::anyhow!("Core Audio probe failed"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn system_control_only_attenuates() {
        assert_eq!(target_reduction(0.1, 0.5, true), 0.0);
        assert!((target_reduction(1.0, 0.5, true) + 6.0206).abs() < 0.001);
        assert_eq!(target_reduction(1.0, 0.5, false), 0.0);
        assert_eq!(target_reduction(f32::NAN, 0.5, true), 0.0);
    }
}
