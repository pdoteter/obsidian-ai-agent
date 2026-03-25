pub mod detect;
pub mod extract;

pub use detect::{detect_urls, DetectedUrl, UrlType};
pub use extract::{fetch_page_content, PageContent};
