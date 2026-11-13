use std::path::{Path, PathBuf};
use std::sync::Once;

use cat_speedup_core::{Transcript, TranscriptSegment};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::{
    AsrEngine, AsrError, AsrOptions, AsrResult, DevicePolicy, ModelSpec, load_wav_16k_mono,
    split_long_segments,
};

/// whisper.cpp based engine. CPU works everywhere; GPU needs a build with
/// `gpu-vulkan` or `gpu-cuda`. `DevicePolicy::Auto` tries GPU and falls back.
pub struct WhisperEngine {
    model_path: PathBuf,
}

impl WhisperEngine {
    pub fn new(spec: &ModelSpec, allow_download: bool) -> Result<Self, AsrError> {
        Ok(Self {
            model_path: spec.resolve(allow_download)?,
        })
    }

    pub fn model_path(&self) -> &Path {
        &self.model_path
    }
}

impl AsrEngine for WhisperEngine {
    fn name(&self) -> &str {
        "whisper.cpp"
    }

    fn transcribe_wav(&mut self, path: &Path, options: &AsrOptions) -> Result<AsrResult, AsrError> {
        let samples = load_wav_16k_mono(path)?;
        install_quiet_logging();

        let gpu_supported = crate::gpu_backend().is_some();
        let (context, used_gpu) = match options.device {
            DevicePolicy::Cpu => (create_context(&self.model_path, false)?, false),
            DevicePolicy::Gpu if !gpu_supported => {
                return Err(AsrError::Engine(
                    "GPU requested, but this binary has no GPU backend; build with --features gpu-vulkan or gpu-cuda"
                        .to_string(),
                ));
            }
            DevicePolicy::Gpu => (create_context(&self.model_path, true)?, true),
            DevicePolicy::Auto if !gpu_supported => {
                (create_context(&self.model_path, false)?, false)
            }
            DevicePolicy::Auto => match create_context(&self.model_path, true) {
                Ok(context) => (context, true),
                Err(_) => (create_context(&self.model_path, false)?, false),
            },
        };

        let mut state = context
            .create_state()
            .map_err(|error| AsrError::Engine(format!("could not create state: {error}")))?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 5 });
        params.set_n_threads(options.threads.max(1) as i32);
        params.set_language(options.language.as_deref());
        params.set_translate(options.translate);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_no_context(false);
        params.set_suppress_blank(true);
        if let Some(prompt) = options.initial_prompt.as_deref() {
            params.set_initial_prompt(prompt);
        }

        state
            .full(params, &samples)
            .map_err(|error| AsrError::Engine(format!("transcription failed: {error}")))?;

        let mut segments = Vec::new();
        for segment in state.as_iter() {
            let text = segment
                .to_str_lossy()
                .map_err(|error| AsrError::Engine(format!("invalid segment text: {error}")))?
                .trim()
                .to_string();

            if text.is_empty() {
                continue;
            }

            let start_ms = segment.start_timestamp().max(0) as u64 * 10;
            let end_ms = segment.end_timestamp().max(0) as u64 * 10;
            if end_ms > start_ms {
                segments.push(TranscriptSegment {
                    start_ms,
                    end_ms,
                    text,
                });
            }
        }

        let transcript = split_long_segments(Transcript { segments }, 32);

        Ok(AsrResult {
            transcript,
            engine: self.name().to_string(),
            model: self.model_path.display().to_string(),
            device: if used_gpu { "gpu" } else { "cpu" }.to_string(),
        })
    }
}

fn create_context(path: &Path, use_gpu: bool) -> Result<WhisperContext, AsrError> {
    let mut params = WhisperContextParameters::default();
    params.use_gpu(use_gpu);
    WhisperContext::new_with_params(path, params).map_err(|error| {
        AsrError::Engine(format!(
            "could not load model {} ({}): {error}",
            path.display(),
            if use_gpu { "gpu" } else { "cpu" }
        ))
    })
}

static QUIET_LOGGING: Once = Once::new();

unsafe extern "C" fn silent_log_callback(
    _level: i32,
    _text: *const std::os::raw::c_char,
    _user_data: *mut std::ffi::c_void,
) {
}

fn install_quiet_logging() {
    QUIET_LOGGING.call_once(|| unsafe {
        whisper_rs::set_log_callback(Some(silent_log_callback), std::ptr::null_mut());
    });
}
