//! Sound files for the time slider: WAV parsing and waveform peaks.

use crate::dag::nodes::SoundData;

/// Peaks per second of the waveform display.
pub const PEAKS_PER_SECOND: usize = 100;

fn u16le(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32le(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// Read a PCM or float WAV file and summarise it as peaks.
pub fn load_wav(path: &str) -> Result<SoundData, String> {
    let b = std::fs::read(path).map_err(|e| format!("Cannot read {}: {}", path, e))?;
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err(format!("{} is not a WAV file (only PCM WAV is supported)", path));
    }
    let (mut fmt, mut data): (Option<(u16, u16, u32, u16)>, Option<(usize, usize)>) = (None, None);
    let mut i = 12;
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let len = u32le(&b, i + 4) as usize;
        let body = i + 8;
        if id == b"fmt " && body + 16 <= b.len() {
            fmt = Some((u16le(&b, body), u16le(&b, body + 2), u32le(&b, body + 4), u16le(&b, body + 14)));
        } else if id == b"data" {
            data = Some((body, len.min(b.len() - body)));
        }
        i = body + len + (len & 1);
    }
    let (format, channels, rate, bits) = fmt.ok_or("The WAV file has no format chunk")?;
    let (start, len) = data.ok_or("The WAV file has no audio data")?;
    if channels == 0 || rate == 0 {
        return Err("The WAV file is damaged".to_string());
    }
    let bytes = (bits / 8) as usize;
    if !matches!((format, bits), (1, 8) | (1, 16) | (1, 24) | (1, 32) | (3, 32)) {
        return Err(format!("Unsupported WAV format (type {}, {} bit): use PCM", format, bits));
    }
    let frame = bytes * channels as usize;
    let frames = len / frame;
    let sample = |idx: usize| -> f32 {
        let o = start + idx * bytes;
        match (format, bits) {
            (1, 8) => (b[o] as f32 - 128.0) / 128.0,
            (1, 16) => i16::from_le_bytes([b[o], b[o + 1]]) as f32 / 32768.0,
            (1, 24) => (((b[o] as i32) << 8 | (b[o + 1] as i32) << 16 | (b[o + 2] as i32) << 24) >> 8) as f32 / 8_388_608.0,
            (1, _) => i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) as f32 / 2_147_483_648.0,
            _ => f32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]),
        }
    };
    let slice = (rate as usize / PEAKS_PER_SECOND).max(1);
    let n = frames.div_ceil(slice);
    let mut peaks = Vec::with_capacity(n);
    for s in 0..n {
        let mut peak = 0.0f32;
        for f in s * slice..((s + 1) * slice).min(frames) {
            for c in 0..channels as usize {
                peak = peak.max(sample(f * channels as usize + c).abs());
            }
        }
        peaks.push(peak.min(1.0));
    }
    Ok(SoundData { path: path.to_string(), sample_rate: rate, channels, seconds: frames as f32 / rate as f32, peaks })
}

/// Write a 16 bit mono WAV (used by tests and to make a click track).
pub fn write_wav(path: &str, rate: u32, samples: &[i16]) -> Result<(), String> {
    let mut out: Vec<u8> = Vec::new();
    let data_len = (samples.len() * 2) as u32;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, out).map_err(|e| format!("Cannot write {}: {}", path, e))
}
