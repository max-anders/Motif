use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use rubato::audioadapter_buffers::direct::SequentialSliceOfVecs;
use rubato::{FixedSync, Resampler};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::TrackType;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

#[derive(Debug, Clone)]
pub struct DecodedAudio {
    pub left: Arc<[f32]>,
    pub right: Arc<[f32]>,
    pub frames: usize,
    pub device_sample_rate: u32,
}

impl DecodedAudio {
    pub fn duration_seconds(&self) -> f32 {
        if self.device_sample_rate == 0 {
            0.0
        } else {
            self.frames as f32 / self.device_sample_rate as f32
        }
    }
}

pub fn decode_audio_file(path: &Path, device_sample_rate: u32) -> Result<DecodedAudio, String> {
    let file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|ext| ext.to_str()) {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|error| format!("probe {}: {error}", path.display()))?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| format!("no default track in {}", path.display()))?;
    let track_id = track.id;
    let codec_params = match track.codec_params.as_ref() {
        Some(symphonia::core::codecs::CodecParameters::Audio(params)) => params,
        _ => return Err(format!("missing audio codec params in {}", path.display())),
    };
    let source_sample_rate = codec_params
        .sample_rate
        .ok_or_else(|| format!("missing sample rate in {}", path.display()))?;

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(codec_params, &AudioDecoderOptions::default())
        .map_err(|error| format!("decoder {}: {error}", path.display()))?;

    let mut left = Vec::<f32>::new();
    let mut right = Vec::<f32>::new();
    let mut interleaved = Vec::<f32>::new();

    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(SymphoniaError::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(SymphoniaError::ResetRequired) => {
                return Err(format!("decoder reset required for {}", path.display()));
            }
            Err(error) => return Err(format!("read packet {}: {error}", path.display())),
        };

        if packet.track_id != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => {
                continue;
            }
            Err(SymphoniaError::ResetRequired) => {
                return Err(format!("decoder reset required for {}", path.display()));
            }
            Err(error) => return Err(format!("decode packet {}: {error}", path.display())),
        };

        let channels = decoded.spec().channels().count();
        if channels == 0 {
            continue;
        }
        let frames = decoded.frames();
        interleaved.clear();
        decoded.copy_to_vec_interleaved(&mut interleaved);

        for frame in 0..frames {
            let base = frame * channels;
            let l = interleaved.get(base).copied().unwrap_or(0.0);
            let r = if channels > 1 {
                interleaved.get(base + 1).copied().unwrap_or(l)
            } else {
                l
            };
            left.push(l);
            right.push(r);
        }
    }

    if left.is_empty() {
        return Err(format!("decoded empty audio from {}", path.display()));
    }

    let (left, right) = if source_sample_rate == device_sample_rate {
        (left, right)
    } else {
        resample_stereo(left, right, source_sample_rate, device_sample_rate)?
    };
    let frames = left.len().min(right.len());

    Ok(DecodedAudio {
        left: Arc::from(left.into_boxed_slice()),
        right: Arc::from(right.into_boxed_slice()),
        frames,
        device_sample_rate,
    })
}

fn resample_stereo(
    left: Vec<f32>,
    right: Vec<f32>,
    source_sample_rate: u32,
    target_sample_rate: u32,
) -> Result<(Vec<f32>, Vec<f32>), String> {
    if left.is_empty() || right.is_empty() {
        return Ok((left, right));
    }
    let frames = left.len().min(right.len());
    let input = vec![left[..frames].to_vec(), right[..frames].to_vec()];

    let mut resampler = rubato::Fft::<f32>::new(
        source_sample_rate as usize,
        target_sample_rate as usize,
        1024,
        2,
        FixedSync::Both,
    )
    .map_err(|error| format!("resampler init failed: {error}"))?;
    let output_capacity = resampler.process_all_needed_output_len(frames);

    let input_adapter = SequentialSliceOfVecs::new(&input, 2, frames)
        .map_err(|error| format!("resampler input adapter: {error}"))?;
    let mut output = vec![vec![0.0_f32; output_capacity], vec![0.0_f32; output_capacity]];
    let mut output_adapter = SequentialSliceOfVecs::new_mut(&mut output, 2, output_capacity)
        .map_err(|error| format!("resampler output adapter: {error}"))?;
    let (_, out_frames) = resampler
        .process_all_into_buffer(&input_adapter, &mut output_adapter, frames, None)
        .map_err(|error| format!("resample failed: {error}"))?;
    output[0].truncate(out_frames);
    output[1].truncate(out_frames);
    Ok((output.remove(0), output.remove(0)))
}
