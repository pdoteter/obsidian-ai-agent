/// Returns a prefix of `s` that is at most `max_bytes` long and ends at a valid UTF-8 character boundary.
pub fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        s
    } else {
        let mut end = max_bytes;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        &s[..end]
    }
}

/// Returns a string containing at most the first `max_chars` characters of `s`.
pub fn truncate_chars(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_truncate_ascii() {
        assert_eq!(safe_truncate("hello", 3), "hel");
        assert_eq!(safe_truncate("hello", 10), "hello");
    }

    #[test]
    fn test_safe_truncate_unicode() {
        let s = "🦀🦀🦀"; // each 🦀 is 4 bytes
        assert_eq!(safe_truncate(s, 0), "");
        assert_eq!(safe_truncate(s, 1), "");
        assert_eq!(safe_truncate(s, 4), "🦀");
        assert_eq!(safe_truncate(s, 7), "🦀");
        assert_eq!(safe_truncate(s, 8), "🦀🦀");
        assert_eq!(safe_truncate(s, 12), "🦀🦀🦀");
    }

    #[test]
    fn test_truncate_chars() {
        let s = "🦀🦀🦀";
        assert_eq!(truncate_chars(s, 2), "🦀🦀");
        assert_eq!(truncate_chars(s, 5), "🦀🦀🦀");
        assert_eq!(truncate_chars("hello", 3), "hel");
    }
}
