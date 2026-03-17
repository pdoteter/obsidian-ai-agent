use std::path::{Path, PathBuf};
use tokio::process::Command;
use tracing::{info, warn};

use crate::error::AudioError;

/// Converts an .oga (Ogg Opus) file to .wav using ffmpeg.
/// Returns the path to the converted .wav file.
pub async fn convert_oga_to_wav(input_path: &Path) -> Result<PathBuf, AudioError> {
    let output_path = input_path.with_extension("wav");

    let output = Command::new("ffmpeg")
        .args([
            "-i",
            input_path.to_str().unwrap_or_default(),
            "-ar", "16000",     // 16kHz sample rate (optimal for speech recognition)
            "-ac", "1",         // Mono channel
            "-sample_fmt", "s16", // 16-bit signed integer
            "-y",               // Overwrite output
            output_path.to_str().unwrap_or_default(),
        ])
        .output()
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AudioError::FfmpegNotFound
            } else {
                AudioError::Io(e)
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!(stderr = %stderr, "ffmpeg conversion failed");
        return Err(AudioError::Conversion(stderr.to_string()));
    }

    let metadata = tokio::fs::metadata(&output_path).await?;
    info!(
        input = %input_path.display(),
        output = %output_path.display(),
        output_size = metadata.len(),
        "Converted audio to WAV"
    );

    Ok(output_path)
}

/// Check if ffmpeg is available in PATH
pub async fn check_ffmpeg() -> Result<(), AudioError> {
    let output = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AudioError::FfmpegNotFound
            } else {
                AudioError::Io(e)
            }
        })?;

    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout);
        let first_line = version.lines().next().unwrap_or("unknown");
        info!(version = %first_line, "ffmpeg found");
        Ok(())
    } else {
        Err(AudioError::FfmpegNotFound)
    }
}
