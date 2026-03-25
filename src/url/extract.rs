use crate::error::UrlError;
use reqwest::header::{CONTENT_TYPE, USER_AGENT};
use scraper::{Html, Selector};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageContent {
    pub title: Option<String>,
    pub description: Option<String>,
    pub body_text: String,
    pub url: String,
}

pub async fn fetch_page_content(
    url: &str,
    timeout_secs: u64,
    max_bytes: usize,
) -> Result<PageContent, UrlError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| UrlError::FetchFailed {
            url: url.to_string(),
            reason: e.to_string(),
        })?;

    let response = client
        .get(url)
        .header(
            USER_AGENT,
            "Mozilla/5.0 (compatible; ObsidianAIAgent/1.0)",
        )
        .send()
        .await
        .map_err(|e| map_reqwest_error(url, timeout_secs, e))?;

    if !response.status().is_success() {
        return Err(UrlError::FetchFailed {
            url: url.to_string(),
            reason: format!("HTTP {}", response.status()),
        });
    }

    if let Some(content_length) = response.content_length() {
        if content_length as usize > max_bytes {
            return Err(UrlError::ContentTooLarge {
                url: url.to_string(),
                size: content_length as usize,
            });
        }
    }

    let is_html = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            let lower = v.to_ascii_lowercase();
            lower.contains("text/html") || lower.contains("application/xhtml+xml")
        })
        .unwrap_or(true);

    if !is_html {
        return Err(UrlError::ParseFailed(
            "Response content type is not HTML".to_string(),
        ));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| map_reqwest_error(url, timeout_secs, e))?;

    if bytes.len() > max_bytes {
        return Err(UrlError::ContentTooLarge {
            url: url.to_string(),
            size: bytes.len(),
        });
    }

    let html_str = String::from_utf8_lossy(&bytes).to_string();
    Ok(extract_page_content_from_html(url, &html_str))
}

fn map_reqwest_error(url: &str, timeout_secs: u64, err: reqwest::Error) -> UrlError {
    if err.is_timeout() {
        UrlError::Timeout {
            url: url.to_string(),
            timeout_secs,
        }
    } else {
        UrlError::FetchFailed {
            url: url.to_string(),
            reason: err.to_string(),
        }
    }
}

fn extract_page_content_from_html(url: &str, html: &str) -> PageContent {
    let doc = Html::parse_document(html);

    let title = extract_meta_content(&doc, "meta[property='og:title']")
        .or_else(|| extract_meta_content(&doc, "meta[name='og:title']"))
        .or_else(|| extract_element_text(&doc, "title"));

    let description = extract_meta_content(&doc, "meta[name='description']")
        .or_else(|| extract_meta_content(&doc, "meta[property='og:description']"));

    let body_text = extract_body_text(&doc);

    PageContent {
        title,
        description,
        body_text,
        url: url.to_string(),
    }
}

fn extract_meta_content(doc: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    doc.select(&selector)
        .next()
        .and_then(|el| el.value().attr("content"))
        .map(normalize_text)
        .filter(|s| !s.is_empty())
}

fn extract_element_text(doc: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    doc.select(&selector)
        .next()
        .map(|el| el.text().collect::<Vec<_>>().join(" "))
        .map(|s| normalize_text(&s))
        .filter(|s| !s.is_empty())
}

fn extract_body_text(doc: &Html) -> String {
    let paragraph_selector = Selector::parse("main p, article p, p").expect("valid selector");
    let container_selector = Selector::parse("main, article").expect("valid selector");
    let body_selector = Selector::parse("body").expect("valid selector");

    let mut pieces = doc
        .select(&paragraph_selector)
        .map(|el| el.text().collect::<String>())
        .map(|s| normalize_text(&s))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();

    if pieces.is_empty() {
        pieces = doc
            .select(&container_selector)
            .map(|el| el.text().collect::<Vec<_>>().join(" "))
            .map(|s| normalize_text(&s))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
    }

    if pieces.is_empty() {
        pieces = doc
            .select(&body_selector)
            .map(|el| el.text().collect::<Vec<_>>().join(" "))
            .map(|s| normalize_text(&s))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
    }

    pieces.join(" ")
}

fn normalize_text(input: &str) -> String {
    input
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::time::{sleep, Duration};

    async fn spawn_server(raw_response: String, response_delay: Option<Duration>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();

            let mut request_buffer = [0_u8; 2048];
            let _ = stream.read(&mut request_buffer).await;

            if let Some(delay) = response_delay {
                sleep(delay).await;
            }

            let _ = stream.write_all(raw_response.as_bytes()).await;
            let _ = stream.shutdown().await;
        });

        format!("http://{}", addr)
    }

    fn html_response(status_line: &str, body: &str) -> String {
        format!(
            "{status_line}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.as_bytes().len()
        )
    }

    #[tokio::test]
    async fn extracts_html_title_tag() {
        let html = "<html><head><title>Example Title</title></head><body><p>Hello</p></body></html>";
        let url = spawn_server(html_response("HTTP/1.1 200 OK", html), None).await;

        let content = fetch_page_content(&url, 5, 1024 * 32).await.unwrap();
        assert_eq!(content.title, Some("Example Title".to_string()));
    }

    #[tokio::test]
    async fn prefers_og_title_over_html_title() {
        let html = r#"<html><head>
            <title>Regular Title</title>
            <meta property="og:title" content="OG Preferred Title" />
        </head><body><p>Hello</p></body></html>"#;
        let url = spawn_server(html_response("HTTP/1.1 200 OK", html), None).await;

        let content = fetch_page_content(&url, 5, 1024 * 32).await.unwrap();
        assert_eq!(content.title, Some("OG Preferred Title".to_string()));
    }

    #[tokio::test]
    async fn extracts_meta_description() {
        let html = r#"<html><head>
            <meta name="description" content="A short summary" />
        </head><body><p>Hello</p></body></html>"#;
        let url = spawn_server(html_response("HTTP/1.1 200 OK", html), None).await;

        let content = fetch_page_content(&url, 5, 1024 * 32).await.unwrap();
        assert_eq!(content.description, Some("A short summary".to_string()));
    }

    #[tokio::test]
    async fn extracts_readable_body_text_from_html() {
        let html = r#"<html><head><title>T</title></head><body>
            <header>Top Nav</header>
            <main>
                <h1>Ignored heading for now</h1>
                <p>Hello <strong>world</strong>.</p>
                <p>Second paragraph.</p>
            </main>
            <script>console.log('ignore me')</script>
            <footer>Bottom footer</footer>
        </body></html>"#;
        let url = spawn_server(html_response("HTTP/1.1 200 OK", html), None).await;

        let content = fetch_page_content(&url, 5, 1024 * 32).await.unwrap();
        assert!(content.body_text.contains("Hello world."));
        assert!(content.body_text.contains("Second paragraph."));
        assert!(!content.body_text.contains("console.log"));
    }

    #[tokio::test]
    async fn returns_content_too_large_when_max_bytes_exceeded() {
        let body = "<html><body><p>tiny</p></body></html>";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 999999\r\nConnection: close\r\n\r\n{body}"
        );
        let url = spawn_server(response, None).await;

        let err = fetch_page_content(&url, 5, 100).await.unwrap_err();
        assert!(matches!(err, UrlError::ContentTooLarge { .. }));
    }

    #[tokio::test]
    async fn returns_timeout_when_request_exceeds_deadline() {
        let html = "<html><head><title>Slow Page</title></head><body><p>hello</p></body></html>";
        let url = spawn_server(
            html_response("HTTP/1.1 200 OK", html),
            Some(Duration::from_secs(2)),
        )
        .await;

        let err = fetch_page_content(&url, 1, 1024 * 32).await.unwrap_err();
        assert!(matches!(err, UrlError::Timeout { .. }));
    }

    #[tokio::test]
    async fn returns_fetch_failed_on_http_error_status() {
        let html = "<html><body><h1>Not found</h1></body></html>";
        let url = spawn_server(html_response("HTTP/1.1 404 Not Found", html), None).await;

        let err = fetch_page_content(&url, 5, 1024 * 32).await.unwrap_err();
        assert!(matches!(err, UrlError::FetchFailed { .. }));
    }

    #[tokio::test]
    async fn handles_empty_minimal_html_gracefully() {
        let html = "<html></html>";
        let url = spawn_server(html_response("HTTP/1.1 200 OK", html), None).await;

        let content = fetch_page_content(&url, 5, 1024 * 32).await.unwrap();
        assert_eq!(content.title, None);
        assert_eq!(content.description, None);
        assert!(content.body_text.is_empty());
        assert_eq!(content.url, url);
    }

    #[tokio::test]
    async fn returns_fetch_failed_when_connection_cannot_be_established() {
        let err = fetch_page_content("http://127.0.0.1:9/unreachable", 1, 1024 * 32)
            .await
            .unwrap_err();
        assert!(matches!(err, UrlError::FetchFailed { .. } | UrlError::Timeout { .. }));
    }
}
