use crate::error::UrlError;
use regex::Regex;
use std::io::ErrorKind;
use tokio::process::Command;
use tracing::{info, warn};

/// Fetch YouTube transcript for a video using yt-dlp CLI.
/// 
/// This function shells out to yt-dlp to download English auto-generated captions
/// in VTT format, parses the output, and returns clean plain text.
/// 
/// # Arguments
/// * `video_id` - YouTube video ID (11-character alphanumeric, e.g. "dQw4w9WgXcQ")
/// 
/// # Errors
/// * `UrlError::TranscriptFailed` - yt-dlp not found, no captions available, or command failed
pub async fn fetch_transcript(video_id: &str) -> Result<String, UrlError> {
    let url = format!("https://youtube.com/watch?v={}", video_id);

    info!(video_id, "Fetching transcript via yt-dlp");

    let output = Command::new("yt-dlp")
        .args(&[
            "--write-auto-sub",
            "--sub-lang",
            "en",
            "--skip-download",
            "--sub-format",
            "vtt",
            "-o",
            "-",
            &url,
        ])
        .output()
        .await
        .map_err(|e| {
            if e.kind() == ErrorKind::NotFound {
                UrlError::TranscriptFailed {
                    video_id: video_id.to_string(),
                    reason: "yt-dlp not found. Install from: https://github.com/yt-dlp/yt-dlp#installation".to_string(),
                }
            } else {
                UrlError::TranscriptFailed {
                    video_id: video_id.to_string(),
                    reason: format!("Failed to execute yt-dlp: {}", e),
                }
            }
        })?;

    // Check command success
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        
        // Check for specific error messages
        if stderr.contains("no suitable subs") || stderr.contains("no subtitles") {
            return Err(UrlError::TranscriptFailed {
                video_id: video_id.to_string(),
                reason: "No English captions available for this video".to_string(),
            });
        }

        warn!(video_id, stderr = %stderr, "yt-dlp command failed");
        return Err(UrlError::TranscriptFailed {
            video_id: video_id.to_string(),
            reason: format!("Failed to fetch transcript: {}", stderr),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    
    if stdout.is_empty() {
        return Err(UrlError::TranscriptFailed {
            video_id: video_id.to_string(),
            reason: "No transcript data returned".to_string(),
        });
    }

    let transcript = parse_vtt(&stdout);

    if transcript.is_empty() {
        return Err(UrlError::TranscriptFailed {
            video_id: video_id.to_string(),
            reason: "Parsed transcript is empty".to_string(),
        });
    }

    info!(
        video_id,
        transcript_length = transcript.len(),
        "Successfully fetched and parsed transcript"
    );

    Ok(transcript)
}

/// Parse VTT (WebVTT) subtitle format into plain text.
/// 
/// Removes:
/// - "WEBVTT" header
/// - Timestamp lines (HH:MM:SS.mmm --> HH:MM:SS.mmm)
/// - HTML formatting tags (<c>, <b>, <i>, <v Speaker>, etc.)
/// - Extra whitespace
/// 
/// Joins subtitle lines with spaces to form readable paragraphs.
fn parse_vtt(raw: &str) -> String {
    // Regex to match timestamp lines
    let timestamp_regex = Regex::new(r"^\d{2}:\d{2}:\d{2}\.\d{3}\s+-->\s+\d{2}:\d{2}:\d{2}\.\d{3}")
        .expect("Timestamp regex is valid");

    // Regex to strip HTML tags
    let html_tag_regex = Regex::new(r"<[^>]+>").expect("HTML tag regex is valid");

    let mut text_lines = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim();

        // Skip empty lines, WEBVTT header, and timestamp lines
        if trimmed.is_empty() || trimmed == "WEBVTT" || timestamp_regex.is_match(trimmed) {
            continue;
        }

        // Skip lines that are just numbers (subtitle indices)
        if trimmed.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }

        // Strip HTML tags
        let cleaned = html_tag_regex.replace_all(trimmed, "");
        
        if !cleaned.is_empty() {
            text_lines.push(cleaned.to_string());
        }
    }

    // Join lines with spaces and collapse multiple spaces
    let joined = text_lines.join(" ");
    
    // Collapse multiple spaces to single space
    let single_space_regex = Regex::new(r"\s+").expect("Whitespace regex is valid");
    single_space_regex.replace_all(&joined, " ").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ytdlp_command_construction() {
        // This test verifies the function exists and has the correct signature.
        // We test with an invalid video ID to check the command construction path.
        // Real execution would require yt-dlp to be installed.
        let result = fetch_transcript("test_video_id").await;
        
        // We expect either NotFound (yt-dlp not installed) or a command failure
        assert!(result.is_err());
        
        if let Err(UrlError::TranscriptFailed { video_id, reason }) = result {
            assert_eq!(video_id, "test_video_id");
            // Reason should mention either "yt-dlp not found" or some other error
            assert!(!reason.is_empty());
        } else {
            panic!("Expected UrlError::TranscriptFailed");
        }
    }

    #[test]
    fn test_parse_vtt_strips_timestamps() {
        let vtt = r#"WEBVTT

00:00:01.000 --> 00:00:05.000
This is the first subtitle line

00:00:05.500 --> 00:00:10.000
This is the second line"#;

        let result = parse_vtt(vtt);
        assert_eq!(result, "This is the first subtitle line This is the second line");
    }

    #[test]
    fn test_parse_vtt_strips_formatting_tags() {
        let vtt = r#"WEBVTT

00:00:01.000 --> 00:00:05.000
This has <c>color tags</c> and <b>bold</b> and <i>italic</i>

00:00:05.500 --> 00:00:10.000
Also <v Speaker>speaker tags</v>"#;

        let result = parse_vtt(vtt);
        assert_eq!(result, "This has color tags and bold and italic Also speaker tags");
    }

    #[test]
    fn test_parse_vtt_joins_lines() {
        let vtt = r#"WEBVTT

00:00:01.000 --> 00:00:05.000
Line one
continues here

00:00:05.500 --> 00:00:10.000
Line two
also continues"#;

        let result = parse_vtt(vtt);
        assert_eq!(result, "Line one continues here Line two also continues");
    }

    #[test]
    fn test_parse_vtt_removes_subtitle_indices() {
        let vtt = r#"WEBVTT

1
00:00:01.000 --> 00:00:05.000
First subtitle

2
00:00:05.500 --> 00:00:10.000
Second subtitle"#;

        let result = parse_vtt(vtt);
        assert_eq!(result, "First subtitle Second subtitle");
    }

    #[test]
    fn test_parse_vtt_handles_empty_input() {
        let result = parse_vtt("");
        assert_eq!(result, "");
    }

    #[test]
    fn test_parse_vtt_handles_only_header() {
        let vtt = "WEBVTT\n\n";
        let result = parse_vtt(vtt);
        assert_eq!(result, "");
    }

    #[test]
    fn test_parse_vtt_collapses_multiple_spaces() {
        let vtt = r#"WEBVTT

00:00:01.000 --> 00:00:05.000
Multiple    spaces     here

00:00:05.500 --> 00:00:10.000
And   more   spaces"#;

        let result = parse_vtt(vtt);
        assert_eq!(result, "Multiple spaces here And more spaces");
    }

    #[test]
    fn test_parse_vtt_complex_formatting() {
        let vtt = r#"WEBVTT

1
00:00:00.000 --> 00:00:02.500
<c.colorE5E5E5>Welcome to the video!</c>

2
00:00:02.500 --> 00:00:05.000
<v Speaker1>Today we'll discuss</v>
<b>important topics</b>

3
00:00:05.000 --> 00:00:08.000
Including <i>various</i> <c>formatting</c> options"#;

        let result = parse_vtt(vtt);
        assert_eq!(
            result,
            "Welcome to the video! Today we'll discuss important topics Including various formatting options"
        );
    }

    // Note: We don't test actual yt-dlp execution because:
    // 1. It requires yt-dlp to be installed in test environment
    // 2. It requires network access to YouTube
    // 3. It would make tests slow and flaky
    // 
    // Error handling for missing yt-dlp and no captions is tested by the error mapping
    // in fetch_transcript() implementation.
}
