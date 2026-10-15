//! How fast the cat spins at each animation tick.
//!
//! With a sound clip, the clip is split into its bursts ("takes"); each spins
//! with its loudness, faster for quick takes and slower for drawn-out ones.
//! Without one, the GIF's own stand/spin pattern is used.

use std::fs::File;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Below this (after normalizing), the cat stands still.
const STAND_BELOW: f32 = 0.2;

/// Full speed while the GIF spins, standing while it holds still.
pub fn from_gif(plan: &crate::cat::Plan) -> Vec<f32> {
    let n = plan.sequence.len();
    (0..n)
        .map(|tick| {
            let previous = plan.sequence[(tick + n - 1) % n];
            if plan.sequence[tick] == previous { 0.0 } else { 1.0 }
        })
        .collect()
}

/// One frame per tick: `stand` at speed 0, otherwise stepping through `spin`
/// by up to `max_step` poses per tick.
pub fn sequence(stand: &str, spin: &[String], max_step: f32, rhythm: &[f32]) -> Vec<String> {
    let mut phase = 0.0f32;
    rhythm
        .iter()
        .map(|&speed| {
            if speed <= 0.0 || spin.is_empty() {
                stand.to_string()
            } else {
                phase += speed * max_step;
                spin[phase as usize % spin.len()].clone()
            }
        })
        .collect()
}

/// One "oiiaioiiiai" in the clip: a loud stretch between pauses.
pub struct Burst {
    /// Sample range in the decoded clip, padded a little on both sides.
    pub samples: std::ops::Range<usize>,
    /// Spin speed per tick while it plays (see `from_audio`).
    pub speeds: Vec<f32>,
}

/// Split the clip into its bursts. Each burst's spin speed follows its
/// loudness, scaled by its tempo: the longest take is normal speed, and a take
/// that says the same thing in less time is a fast version and spins faster.
pub fn bursts(samples: &[f32], rate: u32, tick_ms: u32) -> Vec<Burst> {
    let rate = rate as usize;
    let window = (rate / 100).max(1); // 10 ms
    let loudness: Vec<f32> = samples
        .chunks(window)
        .map(|c| (c.iter().map(|s| s * s).sum::<f32>() / c.len() as f32).sqrt())
        .collect();
    if loudness.is_empty() {
        return Vec::new();
    }
    let mut sorted = loudness.clone();
    sorted.sort_by(f32::total_cmp);
    let floor = sorted[sorted.len() / 5];
    let peak = sorted[sorted.len() * 95 / 100];
    let span = (peak - floor).max(1e-6);
    let loud = floor + span * STAND_BELOW;

    // Loud stretches, bridging dips shorter than 150 ms between syllables.
    let mut ranges = Vec::new();
    let mut start = None;
    let mut quiet = 0;
    for (i, &level) in loudness.iter().enumerate() {
        if level > loud {
            start.get_or_insert(i);
            quiet = 0;
        } else if let Some(s) = start {
            quiet += 1;
            if quiet > 15 {
                ranges.push(s..i - quiet + 1);
                start = None;
            }
        }
    }
    if let Some(s) = start {
        ranges.push(s..loudness.len());
    }
    // Drop blips; a real burst lasts a good fraction of a second.
    ranges.retain(|r| r.len() >= 30);
    // Clips often repeat a take; keep one per speed (lengths within 10%).
    let mut kept: Vec<std::ops::Range<usize>> = Vec::new();
    for range in ranges {
        let len = range.len() as f32;
        if !kept.iter().any(|k| (k.len() as f32 - len).abs() <= len * 0.1) {
            kept.push(range);
        }
    }
    let ranges = kept;
    if ranges.is_empty() {
        return Vec::new();
    }

    let normal = ranges.iter().map(|r| r.len()).max().unwrap_or(1) as f32;
    let per_tick = (tick_ms as usize / 10).max(1);
    ranges
        .into_iter()
        .map(|range| {
            // Squared so fast takes stand out visibly.
            let tempo = (normal / range.len() as f32).powi(2);
            // 60 ms lead-in, 150 ms tail so syllables are not clipped.
            let first = range.start.saturating_sub(6);
            let last = (range.end + 15).min(loudness.len());
            let speeds = loudness[first..last]
                .chunks(per_tick)
                .map(|chunk| {
                    let level = chunk.iter().sum::<f32>() / chunk.len() as f32;
                    let speed = ((level - floor) / span).clamp(0.0, 1.0);
                    if speed < STAND_BELOW { 0.0 } else { speed * tempo }
                })
                .collect();
            Burst { samples: first * window..(last * window).min(samples.len()), speeds }
        })
        .collect()
}

/// Write mono samples as a 16-bit PCM WAV, which every player understands.
pub fn write_wav(path: &Path, samples: &[f32], rate: u32) -> std::io::Result<()> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + samples.len() * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&((sample.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, out)
}

pub fn decode_mono(path: &Path) -> Option<(Vec<f32>, u32)> {
    let source = MediaSourceStream::new(Box::new(File::open(path).ok()?), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, source, &FormatOptions::default(), &MetadataOptions::default())
        .ok()?;
    let mut format = probed.format;
    let track = format.default_track()?;
    let track_id = track.id;
    let rate = track.codec_params.sample_rate?;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .ok()?;

    let mut mono = Vec::new();
    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }
        let Ok(decoded) = decoder.decode(&packet) else { continue };
        let channels = decoded.spec().channels.count().max(1);
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
        buffer.copy_interleaved_ref(decoded);
        mono.extend(
            buffer
                .samples()
                .chunks(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32),
        );
    }
    (!mono.is_empty()).then_some((mono, rate))
}
