pub mod detect;
pub mod extract;
pub mod transcript;
pub mod youtube;

pub use detect::{detect_urls, DetectedUrl, UrlType, is_transcript_request};
pub use extract::{fetch_page_content, PageContent};
pub use transcript::fetch_transcript;
pub use youtube::{fetch_youtube_metadata, fetch_youtube_description, YouTubeMetadata};
