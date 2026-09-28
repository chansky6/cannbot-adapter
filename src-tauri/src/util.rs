//! Small shared string helpers.

/// Truncate a string to at most `max` bytes, ending with `…`, never panicking
/// on multi-byte UTF-8 boundaries (upstream error bodies are often Chinese).
/// The trailing ellipsis may push the result past `max` by up to 3 bytes.
pub fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_short_is_identity() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("", 5), "");
    }

    #[test]
    fn truncate_ascii() {
        assert_eq!(truncate("hello world", 5), "hello…");
    }

    #[test]
    fn truncate_multibyte_never_panics_and_stays_valid() {
        // 100 CJK chars = 300 bytes; cut points land mid-char for max % 3 != 0.
        let s = "配".repeat(100);
        for max in [199usize, 200, 201, 202, 250, 299] {
            let out = truncate(&s, max);
            assert!(out.len() <= max + 3, "max={max} len={}", out.len());
            // ends with the ellipsis and the rest is a valid char sequence
            assert!(out.ends_with('…'));
            assert!(out.strip_suffix('…').unwrap().chars().all(|c| c == '配'));
        }
    }

    #[test]
    fn truncate_max_inside_first_char_keeps_ellipsis_only() {
        // max=1 falls inside the 3-byte '配'; must not panic.
        assert_eq!(truncate("配配", 1), "…");
    }
}
