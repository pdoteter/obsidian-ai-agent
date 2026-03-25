pub mod detect;
pub mod extract;
pub mod youtube;

pub use detect::{detect_urls, DetectedUrl, UrlType};
pub use extract::{fetch_page_content, PageContent};
pub use youtube::{fetch_youtube_metadata, YouTubeMetadata};
