use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::Voice;
use tracing::info;

use crate::error::AudioError;

/// Downloads a voice message to an in-memory buffer.
#[allow(dead_code)]
pub async fn download_voice_to_memory(bot: &Bot, voice: &Voice) -> Result<Vec<u8>, AudioError> {
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
