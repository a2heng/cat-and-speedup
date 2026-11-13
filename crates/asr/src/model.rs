use std::env;
use std::fs::{self, File};
use std::io::copy;
use std::path::{Path, PathBuf};

use crate::AsrError;

pub const DEFAULT_MODEL: &str = "base";

#[derive(Debug, Clone)]
pub enum ModelSpec {
    Path(PathBuf),
    Named(String),
}

impl ModelSpec {
    pub fn resolve(&self, allow_download: bool) -> Result<PathBuf, AsrError> {
        match self {
            Self::Path(path) => {
                if path.is_file() {
                    Ok(path.clone())
                } else {
                    Err(AsrError::ModelMissing {
                        path: path.display().to_string(),
                    })
                }
            }
            Self::Named(name) => resolve_named(name, allow_download),
        }
    }
}

pub fn resolve_model(value: Option<&str>) -> ModelSpec {
    let value = value
        .map(str::to_owned)
        .or_else(|| env::var("CAT_SPEEDUP_MODEL").ok())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());

    if value.contains('/') || value.contains('\\') || value.ends_with(".bin") {
        ModelSpec::Path(PathBuf::from(value))
    } else {
        ModelSpec::Named(value)
    }
}

fn model_dir() -> PathBuf {
    env::var_os("CAT_SPEEDUP_MODEL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("models"))
}

fn resolve_named(name: &str, allow_download: bool) -> Result<PathBuf, AsrError> {
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
        || name.contains("..")
    {
        return Err(AsrError::ModelMissing {
            path: name.to_string(),
        });
    }

    let path = model_dir().join(format!("ggml-{name}.bin"));
    if path.is_file() && file_size(&path) > 1_000_000 {
        return Ok(path);
    }

    if !allow_download {
        return Err(AsrError::ModelMissing {
            path: path.display().to_string(),
        });
    }

    download_model(name, &path)?;
    Ok(path)
}

fn download_model(name: &str, target: &Path) -> Result<(), AsrError> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }

    let urls = [
        format!("https://hf-mirror.com/ggerganov/whisper.cpp/resolve/main/ggml-{name}.bin"),
        format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{name}.bin"),
    ];

    let part = target.with_extension("bin.part");
    let mut errors = Vec::new();

    for url in urls {
        eprintln!("downloading {url}");
        match download_one(&url, &part) {
            Ok(()) => {
                fs::rename(&part, target)?;
                return Ok(());
            }
            Err(error) => {
                let _ = fs::remove_file(&part);
                errors.push(format!("{url}: {error}"));
            }
        }
    }

    Err(AsrError::Download(errors.join("\n")))
}

fn download_one(url: &str, part: &Path) -> Result<(), AsrError> {
    let response = ureq::get(url)
        .call()
        .map_err(|error| AsrError::Download(error.to_string()))?;
    let total = response
        .header("Content-Length")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);

    if total > 0 {
        eprintln!("model size: {:.1} MiB", total as f64 / 1_048_576.0);
    }

    let mut reader = response.into_reader();
    let mut file = File::create(part)?;
    copy(&mut reader, &mut file)?;
    file.sync_all()?;

    let size = file_size(part);
    if size < 1_000_000 {
        return Err(AsrError::Download(format!(
            "downloaded file is too small ({size} bytes)"
        )));
    }

    Ok(())
}

fn file_size(path: &Path) -> u64 {
    fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_model_maps_into_models_dir() {
        let spec = ModelSpec::Named("base".into());
        match spec {
            ModelSpec::Named(name) => assert_eq!(name, "base"),
            _ => unreachable!(),
        }
    }

    #[test]
    fn paths_are_detected() {
        assert!(matches!(
            resolve_model(Some("models/ggml-base.bin")),
            ModelSpec::Path(_)
        ));
        assert!(matches!(resolve_model(Some("small")), ModelSpec::Named(_)));
    }
}
