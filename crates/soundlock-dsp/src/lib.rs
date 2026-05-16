//! Stereo-linked leveling, compression and a 5 ms look-ahead sample-peak limiter.
//! All memory is allocated in `Processor::new`, never in `process`.
use std::collections::VecDeque;

pub type Frame = [f32; 2];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Profile {
    #[default]
    Night,
    Headphones,
    Speakers,
}

impl Profile {
    fn tuning(self) -> Tuning {
        match self {
            Self::Night => Tuning {
                target: -24.0,
                threshold: -26.0,
                ratio: 6.0,
                lift: 12.0,
                ceiling: -3.0,
            },
            Self::Headphones => Tuning {
                target: -22.0,
                threshold: -20.0,
                ratio: 3.5,
                lift: 9.0,
                ceiling: -1.5,
            },
            Self::Speakers => Tuning {
                target: -20.0,
                threshold: -16.0,
                ratio: 2.5,
                lift: 6.0,
                ceiling: -1.0,
            },
        }
    }
}

struct Tuning {
    target: f32,
    threshold: f32,
    ratio: f32,
    lift: f32,
    ceiling: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub profile: Profile,
    /// Linear amplitude multiplier: 1.0 = 100%, 4.0 = 400% (+12.04 dB).
    pub boost: f32,
    /// Disable leveling and manual gain while keeping the protective limiter.
    pub enabled: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            profile: Profile::Night,
            boost: 1.0,
            enabled: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Meters {
    pub input_peak: f32,
    pub output_peak: f32,
    pub reduction_db: f32,
}

pub struct Processor {
    energy: f32,
    rms_coeff: f32,
    gain_db: f32,
    attack: f32,
    release: f32,
    limiter_release: f32,
    limiter_gain: f32,
    delay: Vec<Frame>,
    position: usize,
    sequence: u64,
    maxima: VecDeque<(u64, f32)>,
    meters: Meters,
}

pub fn db_to_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}
pub fn gain_to_db(gain: f32) -> f32 {
    20.0 * gain.max(1e-6).log10()
}
fn coefficient(ms: f32, sample_rate: f32) -> f32 {
    (-1.0 / (ms * 0.001 * sample_rate)).exp()
}
fn clean(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-16.0, 16.0)
    } else {
        0.0
    }
}
fn peak(frame: Frame) -> f32 {
    frame[0].abs().max(frame[1].abs())
}

impl Processor {
    pub fn new(sample_rate: u32) -> Self {
        let rate = sample_rate.clamp(8_000, 384_000) as f32;
        let lookahead = (rate * 0.005).round() as usize;
        Self {
            energy: 0.0,
            rms_coeff: coefficient(25.0, rate),
            gain_db: 0.0,
            attack: coefficient(8.0, rate),
            release: coefficient(300.0, rate),
            limiter_release: coefficient(90.0, rate),
            limiter_gain: 1.0,
            delay: vec![[0.0; 2]; lookahead],
            position: 0,
            sequence: 0,
            maxima: VecDeque::with_capacity(lookahead + 2),
            meters: Meters::default(),
        }
    }

    pub fn latency_frames(&self) -> usize {
        self.delay.len()
    }
    pub fn meters(&self) -> Meters {
        self.meters
    }

    pub fn process(&mut self, input: Frame, settings: Settings) -> Frame {
        let input = input.map(clean);
        let tuning = settings.profile.tuning();
        // Link using the louder channel so panned speech gets the same dynamics.
        let input_peak = peak(input);
        self.energy = self.rms_coeff * self.energy + (1.0 - self.rms_coeff) * input_peak.powi(2);
        let rms_db = gain_to_db(self.energy.sqrt());
        // Fade upward leveling out below -42 dBFS; never chase digital silence/noise.
        let gate = ((rms_db + 55.0) / 13.0).clamp(0.0, 1.0);
        let lift = (tuning.target - rms_db).clamp(0.0, tuning.lift) * gate;
        // 6 dB soft knee.
        let over = rms_db - tuning.threshold;
        let knee = if over < -3.0 {
            0.0
        } else if over > 3.0 {
            over
        } else {
            (over + 3.0).powi(2) / 12.0
        };
        let compression = knee * (1.0 - 1.0 / tuning.ratio);
        let boost = if settings.boost.is_finite() {
            settings.boost.clamp(1.0, 4.0)
        } else {
            1.0
        };
        let target = if settings.enabled {
            lift - compression + gain_to_db(boost)
        } else {
            0.0
        };
        let coeff = if target < self.gain_db {
            self.attack
        } else {
            self.release
        };
        self.gain_db = coeff * self.gain_db + (1.0 - coeff) * target;
        let frame = input.map(|x| x * db_to_gain(self.gain_db));

        // Sliding maximum covers the delayed sample AND every sample ahead of it.
        // Instant gain attack prevents even a one-sample transient exceeding ceiling.
        let index = self.sequence;
        self.sequence += 1;
        let oldest = index.saturating_sub(self.delay.len() as u64);
        while self.maxima.front().is_some_and(|&(i, _)| i < oldest) {
            self.maxima.pop_front();
        }
        let frame_peak = peak(frame);
        while self.maxima.back().is_some_and(|&(_, p)| p <= frame_peak) {
            self.maxima.pop_back();
        }
        self.maxima.push_back((index, frame_peak));
        let ceiling = db_to_gain(if settings.enabled {
            tuning.ceiling
        } else {
            -1.0
        });
        let max_peak = self.maxima.front().map_or(0.0, |&(_, p)| p);
        let wanted = (ceiling / max_peak.max(1e-10)).min(1.0);
        self.limiter_gain =
            wanted.min(self.limiter_release * self.limiter_gain + (1.0 - self.limiter_release));
        let delayed = std::mem::replace(&mut self.delay[self.position], frame);
        self.position = (self.position + 1) % self.delay.len();
        let output = delayed.map(|x| (x * self.limiter_gain).clamp(-ceiling, ceiling));
        self.meters = Meters {
            input_peak,
            output_peak: peak(output),
            reduction_db: compression.max(0.0) - gain_to_db(self.limiter_gain),
        };
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const RATE: u32 = 48_000;
    fn sine(i: usize, amplitude: f32) -> Frame {
        let s = (i as f32 * 440.0 * std::f32::consts::TAU / RATE as f32).sin() * amplitude;
        [s, s * 0.5]
    }
    fn steady_rms(amplitude: f32, settings: Settings) -> f32 {
        let mut p = Processor::new(RATE);
        let mut energy = 0.0;
        for i in 0..RATE as usize * 3 {
            let out = p.process(sine(i, amplitude), settings);
            if i >= RATE as usize * 2 {
                energy += out[0] * out[0];
            }
        }
        (energy / RATE as f32).sqrt()
    }
    #[test]
    fn silence_stays_silent() {
        let mut p = Processor::new(RATE);
        for _ in 0..RATE {
            assert_eq!(
                p.process(
                    [0.0; 2],
                    Settings {
                        boost: 4.0,
                        ..Default::default()
                    }
                ),
                [0.0; 2]
            );
        }
    }
    #[test]
    fn isolated_transient_never_escapes_lookahead() {
        for profile in [Profile::Night, Profile::Headphones, Profile::Speakers] {
            let mut p = Processor::new(RATE);
            let settings = Settings {
                profile,
                boost: 4.0,
                enabled: true,
            };
            for i in 0..100_000 {
                let input = if i == 50_000 {
                    [8.0, -4.0]
                } else {
                    sine(i, 0.02)
                };
                let out = p.process(input, settings);
                assert!(peak(out) <= db_to_gain(profile.tuning().ceiling) + 1e-6);
            }
        }
    }
    #[test]
    fn stereo_image_is_preserved() {
        let mut p = Processor::new(RATE);
        for i in 0..100_000 {
            let out = p.process(
                sine(i, 0.8),
                Settings {
                    boost: 4.0,
                    ..Default::default()
                },
            );
            assert!((out[0] * 0.5 - out[1]).abs() < 1e-6);
        }
    }
    #[test]
    fn quiet_scenes_are_lifted() {
        let processed = steady_rms(0.02, Settings::default());
        let dry = steady_rms(
            0.02,
            Settings {
                enabled: false,
                ..Default::default()
            },
        );
        assert!(processed > dry * 2.0, "wet {processed}, dry {dry}");
    }
    #[test]
    fn dynamic_range_is_reduced() {
        let quiet = steady_rms(0.02, Settings::default());
        let loud = steady_rms(0.8, Settings::default());
        assert!(loud / quiet < 6.0, "ratio {}", loud / quiet);
    }
    #[test]
    fn boost_is_four_times_before_limiting() {
        let a = steady_rms(
            0.01,
            Settings {
                profile: Profile::Speakers,
                ..Default::default()
            },
        );
        let b = steady_rms(
            0.01,
            Settings {
                profile: Profile::Speakers,
                boost: 4.0,
                enabled: true,
            },
        );
        assert!((b / a - 4.0).abs() < 0.02);
    }
    #[test]
    fn bypass_preserves_delayed_samples_at_unity() {
        let mut p = Processor::new(RATE);
        let delay = p.latency_frames();
        for i in 0..10_000 {
            let out = p.process(
                sine(i, 0.1),
                Settings {
                    enabled: false,
                    ..Default::default()
                },
            );
            let expected = if i < delay {
                [0.0; 2]
            } else {
                sine(i - delay, 0.1)
            };
            assert!((out[0] - expected[0]).abs() < 1e-6);
        }
    }
    #[test]
    fn invalid_samples_and_settings_do_not_poison_state() {
        let mut p = Processor::new(RATE);
        for _ in 0..1000 {
            assert!(
                p.process(
                    [f32::NAN, f32::INFINITY],
                    Settings {
                        boost: f32::NAN,
                        ..Default::default()
                    }
                )
                .iter()
                .all(|v| v.is_finite())
            );
        }
        assert!(steady_rms(0.02, Settings::default()) > 0.0);
        for i in 0..10_000 {
            assert!(
                p.process(sine(i, 0.1), Settings::default())
                    .iter()
                    .all(|v| v.is_finite())
            );
        }
    }
    #[test]
    fn profile_and_gain_changes_remain_bounded() {
        let mut p = Processor::new(RATE);
        for i in 0..100_000 {
            let settings = Settings {
                profile: if i % 2 == 0 {
                    Profile::Night
                } else {
                    Profile::Speakers
                },
                boost: if i % 3 == 0 { 4.0 } else { 1.0 },
                enabled: i % 5 != 0,
            };
            assert!(peak(p.process(sine(i, 1.0), settings)) <= 1.0);
        }
    }
    #[test]
    fn noise_below_gate_is_not_auto_amplified() {
        let a = steady_rms(0.0001, Settings::default());
        let b = steady_rms(
            0.0001,
            Settings {
                enabled: false,
                ..Default::default()
            },
        );
        assert!((a / b - 1.0).abs() < 0.01);
    }
    #[test]
    fn common_sample_rates_have_five_ms_latency() {
        for rate in [8_000, 44_100, 48_000, 96_000, 192_000] {
            assert!(
                (Processor::new(rate).latency_frames() as f32 / rate as f32 - 0.005).abs() < 0.0001
            );
        }
    }
}
