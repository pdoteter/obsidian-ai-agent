pub mod detect;
pub mod extract;
pub mod transcript;
pub mod youtube;

pub use detect::is_transcript_request;
pub use extract::PageContent;
pub use transcript::fetch_transcript;
