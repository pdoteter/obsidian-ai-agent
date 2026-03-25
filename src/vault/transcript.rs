use std::path::{Path, PathBuf};
use tokio::fs;

/// Transcript file information returned after successful save
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptFile {
    pub path: PathBuf,
    pub wiki_link: String,
}

/// Sanitize a string into a filename-safe slug (lowercase, alphanumeric + hyphens, max 50 chars)
fn sanitize_slug(input: &str) -> String {
    let lowercase = input.to_lowercase();
    let alphanumeric: String = lowercase
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let collapsed = alphanumeric
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    collapsed.chars().take(50).collect()
}

/// Save a YouTube transcript to a markdown file in the vault.
///
/// # Arguments
/// * `vault_path` - Root path to the Obsidian vault
/// * `transcript_folder` - Folder name for transcripts (e.g., "transcripts")
/// * `video_id` - YouTube video ID
/// * `title` - Video title for filename slug and markdown heading
/// * `summary` - AI-generated summary
/// * `transcript_text` - Full transcript text
/// * `date` - Date string in YYYY-MM-DD format
///
/// # Returns
/// `TranscriptFile` with the full file path and wiki-link format
pub async fn save_transcript(
    vault_path: &Path,
    transcript_folder: &str,
    video_id: &str,
    title: &str,
    summary: &str,
    transcript_text: &str,
    date: &str,
) -> Result<TranscriptFile, std::io::Error> {
    // Generate filename slug from title
    let slug = sanitize_slug(title);
    let filename = format!("{}-{}.md", date, slug);
    let filename_without_ext = format!("{}-{}", date, slug);

    // Ensure transcript directory exists
    let transcript_dir = vault_path.join(transcript_folder);
    fs::create_dir_all(&transcript_dir).await?;

    // Build markdown content with YAML frontmatter
    let markdown_content = format!(
        r#"---
source: https://youtube.com/watch?v={}
video_id: {}
date: {}
tags: [transcript, youtube]
---

# {}

## Summary
{}

## Transcript
{}"#,
        video_id, video_id, date, title, summary, transcript_text
    );

    // Write file
    let full_path = transcript_dir.join(&filename);
    fs::write(&full_path, markdown_content).await?;

    // Generate wiki-link (Obsidian convention: no .md extension, subdirectory included)
    let wiki_link = format!("[[{}/{}]]", transcript_folder, filename_without_ext);

    Ok(TranscriptFile {
        path: full_path,
        wiki_link,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_transcript_file_path_generation() {
        // Verify path construction logic
        let vault_path = PathBuf::from("/vault");
        let transcript_folder = "transcripts";
        let date = "2026-03-25";
        let slug = "my-cool-video";
        
        let expected_filename = format!("{}-{}.md", date, slug);
        let expected_path = vault_path.join(transcript_folder).join(&expected_filename);
        
        // Path ends correctly (OS-agnostic)
        assert!(expected_path.to_str().unwrap().ends_with("2026-03-25-my-cool-video.md"));
        assert!(expected_path.to_str().unwrap().contains("transcripts"));
    }

    #[tokio::test]
    async fn test_transcript_file_content_structure() {
        // Verify markdown structure components
        let video_id = "dQw4w9WgXcQ";
        let title = "Test Video";
        let summary = "This is a test summary.";
        let transcript_text = "Line 1\nLine 2\nLine 3";
        let date = "2026-03-25";

        let content = format!(
            r#"---
source: https://youtube.com/watch?v={}
video_id: {}
date: {}
tags: [transcript, youtube]
---

# {}

## Summary
{}

## Transcript
{}"#,
            video_id, video_id, date, title, summary, transcript_text
        );

        // Verify frontmatter
        assert!(content.contains("---\n"));
        assert!(content.contains("source: https://youtube.com/watch?v=dQw4w9WgXcQ"));
        assert!(content.contains("video_id: dQw4w9WgXcQ"));
        assert!(content.contains("date: 2026-03-25"));
        assert!(content.contains("tags: [transcript, youtube]"));
        
        // Verify structure
        assert!(content.contains("# Test Video"));
        assert!(content.contains("## Summary"));
        assert!(content.contains("This is a test summary."));
        assert!(content.contains("## Transcript"));
        assert!(content.contains("Line 1\nLine 2\nLine 3"));
    }

    #[test]
    fn test_slug_generation_from_title() {
        assert_eq!(sanitize_slug("My Cool Video!"), "my-cool-video");
        assert_eq!(sanitize_slug("Video: Part 1 (2026)"), "video-part-1-2026");
        assert_eq!(sanitize_slug("Test@#$%Video"), "test-video");
        assert_eq!(sanitize_slug("multiple---dashes"), "multiple-dashes");
        assert_eq!(sanitize_slug("  Leading and trailing  "), "leading-and-trailing");
    }

    #[test]
    fn test_wiki_link_format() {
        let transcript_folder = "transcripts";
        let date = "2026-03-25";
        let slug = "my-video";
        let filename_without_ext = format!("{}-{}", date, slug);
        
        let wiki_link = format!("[[{}/{}]]", transcript_folder, filename_without_ext);
        
        assert_eq!(wiki_link, "[[transcripts/2026-03-25-my-video]]");
        assert!(!wiki_link.contains(".md"));
    }

    #[tokio::test]
    async fn test_transcript_folder_creation() {
        // Test that function attempts directory creation
        let temp_dir = std::env::temp_dir().join("obsidian_test_transcript_dir");
        let transcript_folder = "test_transcripts";
        
        // Clean up if exists
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        
        // Create vault directory
        tokio::fs::create_dir_all(&temp_dir).await.unwrap();
        
        let result = save_transcript(
            &temp_dir,
            transcript_folder,
            "test123",
            "Test Title",
            "Test summary",
            "Test transcript",
            "2026-03-25",
        )
        .await;
        
        assert!(result.is_ok());
        
        // Verify directory was created
        let transcript_dir = temp_dir.join(transcript_folder);
        assert!(transcript_dir.exists());
        
        // Clean up
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    #[test]
    fn test_slug_truncation() {
        let long_title = "A".repeat(100);
        let slug = sanitize_slug(&long_title);
        
        assert_eq!(slug.len(), 50);
        assert_eq!(slug, "a".repeat(50));
    }

    #[tokio::test]
    async fn test_save_transcript_full_integration() {
        let temp_dir = std::env::temp_dir().join("obsidian_test_transcript_full");
        let transcript_folder = "transcripts";
        
        // Clean up if exists
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        
        // Create vault directory
        tokio::fs::create_dir_all(&temp_dir).await.unwrap();
        
        let result = save_transcript(
            &temp_dir,
            transcript_folder,
            "dQw4w9WgXcQ",
            "My Test Video!",
            "This is a summary.",
            "Full transcript text here.",
            "2026-03-25",
        )
        .await;
        
        assert!(result.is_ok());
        
        let transcript_file = result.unwrap();
        
        // Verify path
        assert!(transcript_file.path.to_str().unwrap().contains("2026-03-25-my-test-video.md"));
        
        // Verify wiki-link
        assert_eq!(transcript_file.wiki_link, "[[transcripts/2026-03-25-my-test-video]]");
        
        // Verify file exists
        assert!(transcript_file.path.exists());
        
        // Verify file content
        let content = tokio::fs::read_to_string(&transcript_file.path).await.unwrap();
        assert!(content.contains("source: https://youtube.com/watch?v=dQw4w9WgXcQ"));
        assert!(content.contains("video_id: dQw4w9WgXcQ"));
        assert!(content.contains("date: 2026-03-25"));
        assert!(content.contains("tags: [transcript, youtube]"));
        assert!(content.contains("# My Test Video!"));
        assert!(content.contains("## Summary"));
        assert!(content.contains("This is a summary."));
        assert!(content.contains("## Transcript"));
        assert!(content.contains("Full transcript text here."));
        
        // Clean up
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }

    #[tokio::test]
    async fn test_special_characters_in_title() {
        let temp_dir = std::env::temp_dir().join("obsidian_test_transcript_special");
        let transcript_folder = "transcripts";
        
        // Clean up if exists
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
        
        tokio::fs::create_dir_all(&temp_dir).await.unwrap();
        
        let result = save_transcript(
            &temp_dir,
            transcript_folder,
            "test123",
            "Video: Part 1 (2026) — Special!",
            "Summary",
            "Transcript",
            "2026-03-25",
        )
        .await;
        
        assert!(result.is_ok());
        
        let transcript_file = result.unwrap();
        
        // Verify slug sanitization
        assert!(transcript_file.path.to_str().unwrap().contains("video-part-1-2026-special"));
        assert_eq!(transcript_file.wiki_link, "[[transcripts/2026-03-25-video-part-1-2026-special]]");
        
        // Verify original title preserved in content
        let content = tokio::fs::read_to_string(&transcript_file.path).await.unwrap();
        assert!(content.contains("# Video: Part 1 (2026) — Special!"));
        
        // Clean up
        let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    }
}
