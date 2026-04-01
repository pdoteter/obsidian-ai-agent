use linkify::{LinkFinder, LinkKind};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use url::Url;

/// Type of URL detected
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum UrlType {
    WebPage,
    YouTube { video_id: String },
}

/// A URL detected in text with its type and position
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DetectedUrl {
    pub url: String,
    pub url_type: UrlType,
    pub start: usize,
    pub end: usize,
}

/// Regex to extract YouTube video IDs from various URL formats.
/// Compiled once at first access using LazyLock.
static YOUTUBE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)
        (?:(?:www\.|m\.)?youtube\.com/(?:watch\?v=|embed/|shorts/)|
           youtu\.be/)
        ([A-Za-z0-9_-]+)
        ",
    )
    .expect("YouTube regex is valid")
});

/// Detect URLs in text and classify them
pub fn detect_urls(text: &str) -> Vec<DetectedUrl> {
    let mut finder = LinkFinder::new();
    finder.kinds(&[LinkKind::Url]);

    let mut detected_urls = Vec::new();

    for link in finder.links(text) {
        let link_str = link.as_str();
        let start = link.start();
        let end = link.end();

        // Try to parse as URL and classify it
        let url_type = classify_url(link_str);

        detected_urls.push(DetectedUrl {
            url: link_str.to_string(),
            url_type,
            start,
            end,
        });
    }

    detected_urls
}

/// Classify a URL by checking its domain and extracting metadata
fn classify_url(url_str: &str) -> UrlType {
    if let Ok(parsed_url) = Url::parse(url_str) {
        if let Some(host) = parsed_url.host_str() {
            // Check if host is a YouTube domain
            if is_youtube_domain(host) {
                // Extract video ID using static regex
                if let Some(captures) = YOUTUBE_REGEX.captures(url_str) {
                    if let Some(video_id_match) = captures.get(1) {
                        return UrlType::YouTube {
                            video_id: video_id_match.as_str().to_string(),
                        };
                    }
                }
            }
        }
    }

    UrlType::WebPage
}

/// Check if a host is a YouTube domain
fn is_youtube_domain(host: &str) -> bool {
    matches!(
        host,
        "youtube.com" | "www.youtube.com" | "youtu.be" | "m.youtube.com"
    )
}

/// Check if a message contains a "transcript" keyword (case-insensitive)
pub fn is_transcript_request(text: &str) -> bool {
    text.to_lowercase().contains("transcript")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_url_in_text() {
        let text = "Check out https://example.com for more info";
        let urls = detect_urls(text);

        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].url, "https://example.com");
        assert_eq!(urls[0].url_type, UrlType::WebPage);
        assert_eq!(urls[0].start, 10);
        // linkify returns the end position inclusive of the last character
        assert_eq!(urls[0].end, 29);
    }

    #[test]
    fn test_multiple_urls_in_text() {
        let text = "First https://example.com and second https://test.org links";
        let urls = detect_urls(text);

        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0].url, "https://example.com");
        assert_eq!(urls[0].start, 6);
        assert_eq!(urls[1].url, "https://test.org");
        assert_eq!(urls[1].start, 37);
    }

    #[test]
    fn test_url_with_trailing_punctuation() {
        let text = "Visit https://example.com. More text here!";
        let urls = detect_urls(text);

        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].url, "https://example.com");
        // Should NOT include the trailing period
        assert!(!urls[0].url.ends_with('.'));
    }

    #[test]
    fn test_url_with_query_and_fragment() {
        let text = "Link: https://example.com/path?key=value&foo=bar#section";
        let urls = detect_urls(text);

        assert_eq!(urls.len(), 1);
        assert_eq!(
            urls[0].url,
            "https://example.com/path?key=value&foo=bar#section"
        );
        assert_eq!(urls[0].url_type, UrlType::WebPage);
    }

    #[test]
    fn test_no_urls_in_text() {
        let text = "This is just plain text with no URLs at all";
        let urls = detect_urls(text);

        assert_eq!(urls.len(), 0);
    }

    #[test]
    fn test_youtube_watch_url() {
        let text = "Watch https://www.youtube.com/watch?v=dQw4w9WgXcQ here";
        let urls = detect_urls(text);

        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].url, "https://www.youtube.com/watch?v=dQw4w9WgXcQ");
        assert_eq!(
            urls[0].url_type,
            UrlType::YouTube {
                video_id: "dQw4w9WgXcQ".to_string()
            }
        );
    }

    #[test]
    fn test_youtube_short_url() {
        let text = "Short link: https://youtu.be/dQw4w9WgXcQ";
        let urls = detect_urls(text);

        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].url, "https://youtu.be/dQw4w9WgXcQ");
        assert_eq!(
            urls[0].url_type,
            UrlType::YouTube {
                video_id: "dQw4w9WgXcQ".to_string()
            }
        );
    }

    #[test]
    fn test_youtube_shorts_url() {
        let text = "Shorts: https://www.youtube.com/shorts/dQw4w9WgXcQ";
        let urls = detect_urls(text);

        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].url, "https://www.youtube.com/shorts/dQw4w9WgXcQ");
        assert_eq!(
            urls[0].url_type,
            UrlType::YouTube {
                video_id: "dQw4w9WgXcQ".to_string()
            }
        );
    }

    #[test]
    fn test_youtube_mobile_url() {
        let text = "Mobile: https://m.youtube.com/watch?v=dQw4w9WgXcQ";
        let urls = detect_urls(text);

        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].url, "https://m.youtube.com/watch?v=dQw4w9WgXcQ");
        assert_eq!(
            urls[0].url_type,
            UrlType::YouTube {
                video_id: "dQw4w9WgXcQ".to_string()
            }
        );
    }

    #[test]
    fn test_youtube_embed_url() {
        let text = "Embed: https://www.youtube.com/embed/dQw4w9WgXcQ";
        let urls = detect_urls(text);

        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].url, "https://www.youtube.com/embed/dQw4w9WgXcQ");
        assert_eq!(
            urls[0].url_type,
            UrlType::YouTube {
                video_id: "dQw4w9WgXcQ".to_string()
            }
        );
    }

    #[test]
    fn test_non_youtube_url_containing_youtube_in_path() {
        let text = "Check https://example.com/watch-youtube-videos for tutorials";
        let urls = detect_urls(text);

        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].url, "https://example.com/watch-youtube-videos");
        // Should NOT be classified as YouTube URL
        assert_eq!(urls[0].url_type, UrlType::WebPage);
    }

    #[test]
    fn test_youtube_url_with_additional_params() {
        let text = "https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42s&list=PLxxx";
        let urls = detect_urls(text);

        assert_eq!(urls.len(), 1);
        // Video ID should still be extracted correctly
        assert_eq!(
            urls[0].url_type,
            UrlType::YouTube {
                video_id: "dQw4w9WgXcQ".to_string()
            }
        );
    }

    #[test]
    fn test_multiple_mixed_urls() {
        let text = "Regular https://example.com and YouTube https://youtu.be/abc123XYZ links";
        let urls = detect_urls(text);

        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0].url_type, UrlType::WebPage);
        assert_eq!(
            urls[1].url_type,
            UrlType::YouTube {
                video_id: "abc123XYZ".to_string()
            }
        );
    }

    #[test]
    fn test_transcript_keyword_detected_lowercase() {
        let text = "transcript https://youtube.com/watch?v=abc123";
        assert!(is_transcript_request(text));
    }

    #[test]
    fn test_transcript_keyword_detected_uppercase() {
        let text = "Transcript https://youtu.be/abc123";
        assert!(is_transcript_request(text));
    }

    #[test]
    fn test_transcript_keyword_in_sentence() {
        let text = "get me the transcript for https://youtube.com/watch?v=abc123";
        assert!(is_transcript_request(text));
    }

    #[test]
    fn test_no_transcript_keyword_returns_false() {
        let text = "https://youtube.com/watch?v=abc123";
        assert!(!is_transcript_request(text));
    }

    #[test]
    fn test_transcript_keyword_mixed_case() {
        let text = "TrAnScRiPt https://youtu.be/abc123";
        assert!(is_transcript_request(text));
    }
}
