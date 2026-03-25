use serde_json::json;
use tracing::{info, warn};

use super::client::OpenRouterClient;
use crate::error::AiError;

/// Result of AI analyzing a single conflicted file
#[derive(Debug, Clone)]
pub struct ConflictAnalysis {
    pub summary: String,
    pub recommendation: String,
    pub confidence: String,
}

/// Truncate a string to max_chars, respecting UTF-8 char boundaries
fn truncate_content(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        return s.to_string();
    }
    let mut end = max_chars;
    while !s.is_char_boundary(end) && end > 0 {
        end -= 1;
    }
    format!("{}... (truncated)", &s[..end])
}

/// Parse AI response text into structured ConflictAnalysis
pub fn parse_analysis_response(text: &str) -> ConflictAnalysis {
    let mut summary = String::new();
    let mut recommendation = String::new();
    let mut confidence = String::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("SUMMARY:") {
            summary = value.trim().to_string();
        } else if let Some(value) = trimmed.strip_prefix("RECOMMENDATION:") {
            recommendation = value.trim().to_string();
        } else if let Some(value) = trimmed.strip_prefix("CONFIDENCE:") {
            confidence = value.trim().to_string();
        }
    }

    // If parsing completely failed, return raw text as summary
    if summary.is_empty() && recommendation.is_empty() && confidence.is_empty() {
        return ConflictAnalysis {
            summary: text.trim().to_string(),
            recommendation: String::new(),
            confidence: "unknown".to_string(),
        };
    }

    ConflictAnalysis {
        summary,
        recommendation,
        confidence: if confidence.is_empty() {
            "unknown".to_string()
        } else {
            confidence
        },
    }
}

/// Analyze a single conflicted file using AI
pub async fn analyze_conflict(
    client: &OpenRouterClient,
    model: &str,
    file_name: &str,
    ours_content: &str,
    theirs_content: &str,
    diff: &str,
) -> Result<ConflictAnalysis, AiError> {
    let system_prompt = "You are analyzing a git merge conflict in an Obsidian vault (markdown notes). \
        Explain what's different between the two versions in simple terms. \
        Recommend which version to keep. Be concise — max 3 sentences for summary, \
        1 sentence for recommendation.\n\n\
        Format your response EXACTLY as:\n\
        SUMMARY: <your summary>\n\
        RECOMMENDATION: <your recommendation>\n\
        CONFIDENCE: <high|medium|low>";

    let user_prompt = format!(
        "File: {file_name}\n\n\
         YOUR version (local):\n{ours}\n\n\
         SERVER version (remote):\n{theirs}\n\n\
         Diff:\n{diff}",
        file_name = file_name,
        ours = truncate_content(ours_content, 2000),
        theirs = truncate_content(theirs_content, 2000),
        diff = truncate_content(diff, 1000),
    );

    let body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_prompt }
        ],
        "max_tokens": 512
    });

    let response = client.chat_completion(body).await?;
    let text = OpenRouterClient::extract_content(&response)?;
    Ok(parse_analysis_response(&text))
}

/// Analyze all conflicts in a ConflictInfo and return a combined human-readable analysis string.
/// This is the main entry point called from the debounce loop.
pub async fn analyze_conflicts(
    client: &OpenRouterClient,
    model: &str,
    conflict_info: &crate::git::sync::ConflictInfo,
) -> Result<String, AiError> {
    if conflict_info.files.is_empty() {
        return Ok("No conflicted files to analyze.".to_string());
    }

    let mut analyses = Vec::new();

    for file_name in &conflict_info.files {
        let ours = conflict_info
            .ours_contents
            .get(file_name)
            .map(|s| s.as_str())
            .unwrap_or("");
        let theirs = conflict_info
            .theirs_contents
            .get(file_name)
            .map(|s| s.as_str())
            .unwrap_or("");

        info!(file = %file_name, "Analyzing conflict");

        match analyze_conflict(
            client,
            model,
            file_name,
            ours,
            theirs,
            &conflict_info.diff_output,
        )
        .await
        {
            Ok(analysis) => {
                analyses.push(format!(
                    "📄 **{}**\n{}\n💡 {}\n(confidence: {})",
                    file_name, analysis.summary, analysis.recommendation, analysis.confidence
                ));
            }
            Err(e) => {
                warn!(file = %file_name, error = %e, "AI analysis failed for file");
                analyses.push(format!("📄 **{}**\nAI analysis unavailable: {}", file_name, e));
            }
        }
    }

    Ok(analyses.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_content_short() {
        let result = truncate_content("hello", 100);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_truncate_content_long() {
        let long = "a".repeat(3000);
        let result = truncate_content(&long, 2000);
        assert!(result.len() <= 2020);
        assert!(result.ends_with("... (truncated)"));
    }

    #[test]
    fn test_truncate_content_unicode() {
        let content = "🎵".repeat(600); // 2400 bytes
        let result = truncate_content(&content, 2000);
        assert!(result.ends_with("... (truncated)"));
    }

    #[test]
    fn test_parse_analysis_response_structured() {
        let response = "SUMMARY: The local version has new entries added today.\n\
                        RECOMMENDATION: Keep YOUR version — it has newer content.\n\
                        CONFIDENCE: high";
        let analysis = parse_analysis_response(response);
        assert_eq!(analysis.confidence, "high");
        assert!(analysis.summary.contains("new entries"));
        assert!(analysis.recommendation.contains("YOUR version"));
    }

    #[test]
    fn test_parse_analysis_response_fallback() {
        let response = "This is just some freeform text without the expected format.";
        let analysis = parse_analysis_response(response);
        assert_eq!(analysis.summary, response);
        assert!(analysis.recommendation.is_empty());
        assert_eq!(analysis.confidence, "unknown");
    }
}
