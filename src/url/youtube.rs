use crate::error::UrlError;
use regex::Regex;
use serde::Deserialize;
use std::time::Duration;

/// YouTube video metadata fetched from oEmbed API
#[derive(Debug, Clone)]
pub struct YouTubeMetadata {
    pub title: String,
    pub author: String,
    pub thumbnail_url: Option<String>,
    pub video_id: String,
}

/// Internal struct for deserializing oEmbed JSON response
#[derive(Debug, Deserialize)]
struct OEmbedResponse {
    title: String,
    author_name: String,
    thumbnail_url: Option<String>,
}

/// Fetch YouTube video metadata using the oEmbed API
///
/// This function fetches metadata from YouTube's oEmbed endpoint without requiring
/// an API key or quota. It returns title, author name, thumbnail URL, and video ID.
///
/// # Arguments
/// * `url` - The YouTube video URL (any format: youtube.com/watch, youtu.be, shorts, etc.)
/// * `timeout_secs` - Request timeout in seconds
///
/// # Returns
/// * `Ok(YouTubeMetadata)` - Successfully fetched metadata
/// * `Err(UrlError::Timeout)` - Request timed out
/// * `Err(UrlError::FetchFailed)` - Network error or HTTP non-success status
/// * `Err(UrlError::ParseFailed)` - Failed to parse JSON response or extract video ID
pub async fn fetch_youtube_metadata(
    url: &str,
    timeout_secs: u64,
) -> Result<YouTubeMetadata, UrlError> {
    // 1. Extract video_id from URL
    let video_id = extract_video_id_from_url(url)?;

    // 2. Build oEmbed API URL
    let oembed_url = format!("https://www.youtube.com/oembed?url={}&format=json", url);

    // 3. Create reqwest client with timeout
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| UrlError::FetchFailed {
            url: url.to_string(),
            reason: format!("Failed to build HTTP client: {}", e),
        })?;

    // 4. Send GET request
    let response = client.get(&oembed_url).send().await.map_err(|e| {
        if e.is_timeout() {
            UrlError::Timeout {
                url: url.to_string(),
                timeout_secs,
            }
        } else {
            UrlError::FetchFailed {
                url: url.to_string(),
                reason: format!("HTTP request failed: {}", e),
            }
        }
    })?;

    // 5. Check HTTP status
    if !response.status().is_success() {
        return Err(UrlError::FetchFailed {
            url: url.to_string(),
            reason: format!(
                "HTTP {}: {}",
                response.status(),
                response
                    .status()
                    .canonical_reason()
                    .unwrap_or("Unknown error")
            ),
        });
    }

    // 6. Parse JSON response
    let oembed: OEmbedResponse = response.json().await.map_err(|e| {
        UrlError::ParseFailed(format!("Failed to parse oEmbed JSON: {}", e))
    })?;

    // 7. Build YouTubeMetadata
    Ok(YouTubeMetadata {
        title: oembed.title,
        author: oembed.author_name,
        thumbnail_url: oembed.thumbnail_url,
        video_id,
    })
}

/// Extract video ID from a YouTube URL
fn extract_video_id_from_url(url: &str) -> Result<String, UrlError> {
    // Use regex to extract video ID from various YouTube URL formats
    // Pattern: (?:youtube\.com/(?:watch\?v=|embed/|shorts/)|youtu\.be/)([A-Za-z0-9_-]+)
    let youtube_regex = Regex::new(
        r"(?x)
        (?:(?:www\.|m\.)?youtube\.com/(?:watch\?v=|embed/|shorts/)|
           youtu\.be/)
        ([A-Za-z0-9_-]+)
        ",
    )
    .expect("YouTube regex is valid");

    if let Some(captures) = youtube_regex.captures(url) {
        if let Some(video_id_match) = captures.get(1) {
            return Ok(video_id_match.as_str().to_string());
        }
    }

    Err(UrlError::ParseFailed(format!(
        "Failed to extract video ID from URL: {}",
        url
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_youtube_metadata_struct_creation() {
        // Verify YouTubeMetadata can be instantiated with all fields
        let metadata = YouTubeMetadata {
            title: "Test Video".to_string(),
            author: "Test Author".to_string(),
            thumbnail_url: Some("https://i.ytimg.com/vi/test/hqdefault.jpg".to_string()),
            video_id: "test123".to_string(),
        };

        assert_eq!(metadata.title, "Test Video");
        assert_eq!(metadata.author, "Test Author");
        assert_eq!(
            metadata.thumbnail_url,
            Some("https://i.ytimg.com/vi/test/hqdefault.jpg".to_string())
        );
        assert_eq!(metadata.video_id, "test123");
    }

    #[test]
    fn test_oembed_response_parsing() {
        // Test parsing a sample oEmbed JSON response with thumbnail
        let json_str = r#"{
            "title": "Test Video",
            "author_name": "Test Author",
            "thumbnail_url": "https://i.ytimg.com/vi/test/hqdefault.jpg"
        }"#;

        let response: OEmbedResponse = serde_json::from_str(json_str).unwrap();

        assert_eq!(response.title, "Test Video");
        assert_eq!(response.author_name, "Test Author");
        assert_eq!(
            response.thumbnail_url,
            Some("https://i.ytimg.com/vi/test/hqdefault.jpg".to_string())
        );
    }

    #[test]
    fn test_oembed_response_no_thumbnail() {
        // Test parsing response without thumbnail_url (optional field)
        let json_str = r#"{
            "title": "Test",
            "author_name": "Author"
        }"#;

        let response: OEmbedResponse = serde_json::from_str(json_str).unwrap();

        assert_eq!(response.title, "Test");
        assert_eq!(response.author_name, "Author");
        assert_eq!(response.thumbnail_url, None);
    }

    #[test]
    fn test_video_id_extraction_watch() {
        // Test extracting video ID from youtube.com/watch URL
        let url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
        let video_id = extract_video_id_from_url(url).unwrap();
        assert_eq!(video_id, "dQw4w9WgXcQ");
    }

    #[test]
    fn test_video_id_extraction_youtu_be() {
        // Test extracting video ID from youtu.be short URL
        let url = "https://youtu.be/dQw4w9WgXcQ";
        let video_id = extract_video_id_from_url(url).unwrap();
        assert_eq!(video_id, "dQw4w9WgXcQ");
    }

    #[test]
    fn test_video_id_extraction_shorts() {
        // Test extracting video ID from youtube.com/shorts URL
        let url = "https://www.youtube.com/shorts/abc123XYZ";
        let video_id = extract_video_id_from_url(url).unwrap();
        assert_eq!(video_id, "abc123XYZ");
    }

    #[test]
    fn test_video_id_extraction_mobile() {
        // Test extracting video ID from m.youtube.com URL
        let url = "https://m.youtube.com/watch?v=test_video";
        let video_id = extract_video_id_from_url(url).unwrap();
        assert_eq!(video_id, "test_video");
    }

    #[test]
    fn test_video_id_extraction_embed() {
        // Test extracting video ID from youtube.com/embed URL
        let url = "https://www.youtube.com/embed/embed123";
        let video_id = extract_video_id_from_url(url).unwrap();
        assert_eq!(video_id, "embed123");
    }

    #[test]
    fn test_video_id_extraction_with_params() {
        // Test extracting video ID from URL with additional query params
        let url = "https://www.youtube.com/watch?v=abc123&t=42s&list=PLxxx";
        let video_id = extract_video_id_from_url(url).unwrap();
        assert_eq!(video_id, "abc123");
    }

    #[test]
    fn test_video_id_extraction_invalid_url() {
        // Test error handling for non-YouTube URL
        let url = "https://example.com/not-youtube";
        let result = extract_video_id_from_url(url);
        assert!(result.is_err());
    }

    #[test]
    #[ignore] // Will test in integration — requires real HTTP requests
    fn test_fetch_youtube_metadata_real() {
        // This test would make a real HTTP request to YouTube's oEmbed API
        // Run with: cargo test --ignored
    }
}
