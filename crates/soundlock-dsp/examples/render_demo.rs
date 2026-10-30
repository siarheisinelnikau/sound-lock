//! Deterministic offline dynamics demonstration; no audio device is opened.
use soundlock_dsp::{Processor, Settings, gain_to_db};
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

fn write_wav(path: &Path, samples: &[f32], rate: u32) -> std::io::Result<()> {
    let mut file = BufWriter::new(File::create(path)?);
    let bytes = samples.len() as u32 * 2;
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + bytes).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16_u32.to_le_bytes())?;
    file.write_all(&1_u16.to_le_bytes())?;
    file.write_all(&2_u16.to_le_bytes())?;
    file.write_all(&rate.to_le_bytes())?;
    file.write_all(&(rate * 4).to_le_bytes())?;
    file.write_all(&4_u16.to_le_bytes())?;
    file.write_all(&16_u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&bytes.to_le_bytes())?;
    for sample in samples {
        file.write_all(
            &((sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16).to_le_bytes(),
        )?;
    }
    file.flush()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args()
        .nth(1)
        .unwrap_or_else(|| ".local/demo".into());
    let directory = Path::new(&directory);
    std::fs::create_dir_all(directory)?;
    let rate = 48_000;
    let mut processor = Processor::new(rate);
    let count = rate as usize * 10;
    let mut before = Vec::with_capacity(count * 2);
    let mut after = Vec::with_capacity(count * 2);
    for i in 0..count + processor.latency_frames() {
        let time = i as f32 / rate as f32;
        let amplitude = if (4.0..4.25).contains(&time) || (7.0..7.5).contains(&time) {
            0.95
        } else if !(1.0..=9.0).contains(&time) {
            0.0
        } else {
            0.018
        };
        let signal = (time * 220.0 * std::f32::consts::TAU).sin() * amplitude;
        let input = if i < count {
            [signal, signal * 0.8]
        } else {
            [0.0; 2]
        };
        let output = processor.process(
            input,
            Settings {
                boost: 2.0,
                ..Default::default()
            },
        );
        if i < count {
            before.extend_from_slice(&input);
        }
        if i >= processor.latency_frames() {
            after.extend_from_slice(&output);
        }
    }
    write_wav(&directory.join("before.wav"), &before, rate)?;
    write_wav(&directory.join("after.wav"), &after, rate)?;
    for (label, samples) in [("before", &before), ("after", &after)] {
        let peak = samples.iter().fold(0.0_f32, |p, &s| p.max(s.abs()));
        let quiet = &samples[rate as usize * 2 * 2..rate as usize * 3 * 2];
        let rms = (quiet.iter().map(|s| s * s).sum::<f32>() / quiet.len() as f32).sqrt();
        println!(
            "{label}: sample peak {:.2} dBFS, quiet scene RMS {:.2} dBFS",
            gain_to_db(peak),
            gain_to_db(rms)
        );
    }
    println!("WAV files written to {}", directory.display());
    Ok(())
}
