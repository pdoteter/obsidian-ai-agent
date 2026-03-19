use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::Voice;
use tempfile::TempDir;
use tracing::info;

use crate::error::AudioError;

/// Downloads a voice message from Telegram to a temporary file.
/// Returns the path to the downloaded .oga file and the temp directory (must stay alive).
#[allow(dead_code)]
pub async fn download_voice(
    bot: &Bot,
    voice: &Voice,
) -> Result<(std::path::PathBuf, TempDir), AudioError> {
    let file = bot
        .get_file(&voice.file.id)
        .await
        .map_err(|e| AudioError::Download(e.to_string()))?;

    let tmp_dir = TempDir::new()?;
    let file_path = tmp_dir.path().join(format!(
        "voice_{}.oga",
        uuid::Uuid::new_v4()
    ));

    let mut dst = tokio::fs::File::create(&file_path).await?;
    bot.download_file(&file.path, &mut dst)
        .await
        .map_err(|e| AudioError::Download(e.to_string()))?;

    info!(
        path = %file_path.display(),
        size_bytes = voice.file.size,
        duration_secs = %voice.duration,
        "Downloaded voice message"
    );

    Ok((file_path, tmp_dir))
}

/// Downloads a voice message to an in-memory buffer.
#[allow(dead_code)]
pub async fn download_voice_to_memory(
    bot: &Bot,
    voice: &Voice,
) -> Result<Vec<u8>, AudioError> {
    use futures::StreamExt;

    let file = bot
        .get_file(&voice.file.id)
        .await
        .map_err(|e| AudioError::Download(e.to_string()))?;

    let mut stream = bot.download_file_stream(&file.path);
    let mut buffer = Vec::with_capacity(voice.file.size as usize);

    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| AudioError::Download(e.to_string()))?;
        buffer.extend_from_slice(&bytes);
    }

    info!(
        size_bytes = buffer.len(),
        duration_secs = %voice.duration,
        "Downloaded voice message to memory"
    );

    Ok(buffer)
}
