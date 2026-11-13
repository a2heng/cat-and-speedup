use std::path::Path;

use cut_speedup_core::Transcript;

pub mod http;
pub mod model;
pub mod sherpa;
pub mod subtitle;
#[cfg(feature = "whisper")]
pub mod whisper;

pub use http::HttpAsrEngine;
pub use model::{ModelSpec, resolve_model};
pub use sherpa::{SherpaEngine, SherpaModelKind};
pub use subtitle::split_long_segments;

#[cfg(feature = "whisper")]
pub fn whisper_cpp_version() -> &'static str {
    whisper_rs::WHISPER_CPP_VERSION
}

pub fn gpu_backend() -> Option<&'static str> {
    if cfg!(feature = "gpu-vulkan") {
        Some("vulkan")
    } else if cfg!(feature = "gpu-cuda") {
        Some("cuda")
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevicePolicy {
    /// Try the GPU backend first, then fall back to CPU.
    Auto,
    Cpu,
    Gpu,
}

impl DevicePolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cpu => "cpu",
            Self::Gpu => "gpu",
        }
    }
}

impl std::str::FromStr for DevicePolicy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "cpu" => Ok(Self::Cpu),
            "gpu" | "cuda" | "vulkan" => Ok(Self::Gpu),
            other => Err(format!(
                "unknown device '{other}' (expected auto, cpu or gpu)"
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AsrOptions {
    /// ISO language code, or None for auto-detection.
    pub language: Option<String>,
    pub translate: bool,
    /// Optional decoder prompt, useful for biasing simplified Chinese output.
    pub initial_prompt: Option<String>,
    pub threads: usize,
    pub device: DevicePolicy,
}

impl Default for AsrOptions {
    fn default() -> Self {
        Self {
            language: None,
            translate: false,
            initial_prompt: None,
            threads: std::thread::available_parallelism()
                .map(|value| value.get())
                .unwrap_or(4)
                .min(8),
            device: DevicePolicy::Auto,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AsrResult {
    pub transcript: Transcript,
    pub engine: String,
    pub model: String,
    pub device: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AsrError {
    #[error(
        "ASR model not found at {path}; run with --model <size> to download, e.g. --model base"
    )]
    ModelMissing { path: String },
    #[error("failed to download model: {0}")]
    Download(String),
    #[error("failed to read audio {path}: {message}")]
    Audio { path: String, message: String },
    #[error("audio must be mono 16-bit PCM WAV: {0}")]
    AudioFormat(String),
    #[error("ASR engine failed: {0}")]
    Engine(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub trait AsrEngine {
    fn name(&self) -> &str;
    fn transcribe_wav(&mut self, path: &Path, options: &AsrOptions) -> Result<AsrResult, AsrError>;
}

/// Read a WAV file as 16 kHz mono f32 samples.
#[allow(dead_code)]
pub(crate) fn load_wav_16k_mono(path: &Path) -> Result<Vec<f32>, AsrError> {
    let mut reader = hound::WavReader::open(path).map_err(|error| AsrError::Audio {
        path: path.display().to_string(),
        message: error.to_string(),
    })?;
    let spec = reader.spec();
    if spec.sample_format != hound::SampleFormat::Int || spec.bits_per_sample != 16 {
        return Err(AsrError::AudioFormat(format!(
            "{} is {:?}/{} bit; expected int/16 bit",
            path.display(),
            spec.sample_format,
            spec.bits_per_sample
        )));
    }

    let channels = spec.channels.max(1) as usize;
    let mut mono = Vec::with_capacity(reader.len() as usize / channels);
    let mut frame = 0.0f32;
    let mut channel_index = 0usize;

    for sample in reader.samples::<i16>() {
        let sample = sample.map_err(|error| AsrError::Audio {
            path: path.display().to_string(),
            message: error.to_string(),
        })?;
        frame += sample as f32 / i16::MAX as f32;
        channel_index += 1;

        if channel_index == channels {
            mono.push(frame / channels as f32);
            frame = 0.0;
            channel_index = 0;
        }
    }

    if spec.sample_rate == 16_000 {
        Ok(mono)
    } else {
        Ok(resample_linear(&mono, spec.sample_rate, 16_000))
    }
}

#[allow(dead_code)]
fn resample_linear(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if samples.is_empty() || from == to || from == 0 || to == 0 {
        return samples.to_vec();
    }

    let ratio = from as f64 / to as f64;
    let output_len = ((samples.len() as f64) / ratio).round() as usize;
    let mut output = Vec::with_capacity(output_len);

    for index in 0..output_len {
        let position = index as f64 * ratio;
        let left = position.floor() as usize;
        let right = (left + 1).min(samples.len() - 1);
        let fraction = (position - left as f64) as f32;
        output.push(samples[left] * (1.0 - fraction) + samples[right] * fraction);
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resamples_length() {
        let input = vec![0.0; 48_000];
        let output = resample_linear(&input, 48_000, 16_000);
        assert_eq!(output.len(), 16_000);
    }
}
