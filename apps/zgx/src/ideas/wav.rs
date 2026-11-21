//! A memo's audio as a WAV file any player opens: mono, 16-bit PCM, at the
//! rate the microphone recorded. Written and read here (no dependency for
//! a header of 44 bytes); reading also takes 32-bit float and several
//! channels (mixed to mono), for a file made elsewhere (a test's input).

/// `samples` (-1..1, mono) at `rate` Hz as a WAV file's bytes.
pub fn encode(samples: &[f32], rate: u32) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes()); // bytes a second
    out.extend_from_slice(&2u16.to_le_bytes()); // bytes a frame
    out.extend_from_slice(&16u16.to_le_bytes()); // bits a sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// A WAV file's samples, mixed to mono, and its rate.
pub fn decode(bytes: &[u8]) -> Result<(Vec<f32>, u32), String> {
    let bad = || "This audio file can't be read".to_string();
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad());
    }
    let mut at = 12;
    let mut format: Option<(u16, u16, u32, u16)> = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = at + 8;
        let end = body.saturating_add(size).min(bytes.len());
        if id == b"fmt " && end - body >= 16 {
            let b = &bytes[body..end];
            let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
            let mut tag = u16_at(0);
            // WAVE_FORMAT_EXTENSIBLE: the real tag opens its sub-format.
            if tag == 0xFFFE && b.len() >= 26 {
                tag = u16_at(24);
            }
            format = Some((tag, u16_at(2), u32::from_le_bytes(b[4..8].try_into().unwrap()), u16_at(14)));
        } else if id == b"data" {
            let (tag, channels, rate, bits) = format.ok_or_else(bad)?;
            let channels = channels.max(1) as usize;
            let data = &bytes[body..end];
            let mono: Vec<f32> = match (tag, bits) {
                (1, 16) => data
                    .chunks_exact(2 * channels)
                    .map(|f| f.chunks_exact(2).map(|s| i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0).sum::<f32>() / channels as f32)
                    .collect(),
                (3, 32) => data
                    .chunks_exact(4 * channels)
                    .map(|f| f.chunks_exact(4).map(|s| f32::from_le_bytes([s[0], s[1], s[2], s[3]])).sum::<f32>() / channels as f32)
                    .collect(),
                _ => return Err("Only 16-bit PCM and 32-bit float WAV files can be read".into()),
            };
            if !(8_000..=192_000).contains(&rate) {
                return Err(bad());
            }
            return Ok((mono, rate));
        }
        // (Chunks are padded to an even length.)
        at = body.saturating_add(size).saturating_add(size & 1);
    }
    Err(bad())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_memo_round_trips_to_the_sample() {
        let samples: Vec<f32> = (0..16_000).map(|i| (i as f32 / 40.0).sin() * 0.5).collect();
        let bytes = encode(&samples, 16_000);
        assert_eq!(bytes.len(), 44 + 32_000);
        let (back, rate) = decode(&bytes).unwrap();
        assert_eq!(rate, 16_000);
        assert_eq!(back.len(), samples.len());
        assert!(back.iter().zip(&samples).all(|(a, b)| (a - b).abs() < 1.0 / 16_000.0));
    }

    #[test]
    fn stereo_float_with_extra_chunks_is_mixed_to_mono() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF\0\0\0\0WAVE");
        bytes.extend_from_slice(b"LIST");
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(b"abc\0");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        for v in [3u16, 2] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes.extend_from_slice(&48_000u32.to_le_bytes());
        bytes.extend_from_slice(&(48_000u32 * 8).to_le_bytes());
        bytes.extend_from_slice(&8u16.to_le_bytes());
        bytes.extend_from_slice(&32u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        for v in [0.5f32, 0.25, -1.0, 0.0] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        assert_eq!(decode(&bytes).unwrap(), (vec![0.375, -0.5], 48_000));
    }

    #[test]
    fn garbage_is_refused_not_panicked_on() {
        assert!(decode(b"").is_err());
        assert!(decode(b"RIFF\0\0\0\0WAVEdata\xff\xff\xff\xff").is_err());
        assert!(decode(&encode(&[0.0], 4_000)).is_err());
    }
}
