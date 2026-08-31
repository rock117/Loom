//! Detect URLs in a terminal line for Ctrl+click open-in-browser.

/// Max accepted URL length (guards absurd / malicious terminal output).
pub const MAX_URL_LEN: usize = 2048;

fn is_url_body_char(c: char) -> bool {
    !c.is_control()
        && !c.is_whitespace()
        && !matches!(
            c,
            '<' | '>' | '"' | '\'' | '{' | '}' | '|' | '\\' | '^' | '`' | '⟨' | '⟩'
        )
}

fn is_trailing_punct(c: char) -> bool {
    matches!(
        c,
        '.' | ',' | ';' | ':' | ')' | ']' | '}' | '!' | '?' | '\'' | '"'
    )
}

const SCHEMES: &[&str] = &[
    "https://",
    "http://",
    "ftp://",
    "file://",
    "mailto:",
    "ssh://",
    "git://",
];

pub fn is_safe_open_url(url: &str) -> bool {
    if url.is_empty() || url.len() > MAX_URL_LEN {
        return false;
    }
    if url.chars().any(|c| c.is_control() || c == ' ' || c == '\t') {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    SCHEMES.iter().any(|scheme| lower.starts_with(scheme))
}

/// All URL spans in `line` as inclusive-exclusive Unicode scalar index ranges.
pub fn url_char_spans(line: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    for scheme in SCHEMES {
        let scheme_chars: Vec<char> = scheme.chars().collect();
        let scheme_len = scheme_chars.len();
        let mut i = 0;
        while i + scheme_len <= chars.len() {
            let matches_scheme = chars[i..i + scheme_len]
                .iter()
                .zip(scheme_chars.iter())
                .all(|(a, b)| a.eq_ignore_ascii_case(b));
            if !matches_scheme {
                i += 1;
                continue;
            }
            let start = i;
            let mut end = i + scheme_len;
            while end < chars.len() && is_url_body_char(chars[end]) {
                end += 1;
            }
            while end > start + scheme_len && is_trailing_punct(chars[end - 1]) {
                end -= 1;
            }
            if end > start + MAX_URL_LEN {
                i = end.max(i + 1);
                continue;
            }
            let url: String = chars[start..end].iter().collect();
            if is_safe_open_url(&url) {
                out.push((start, end));
            }
            i = end.max(i + 1);
        }
    }
    out.sort_by_key(|(s, _)| *s);
    out
}

pub fn url_covering_char(line: &str, char_idx: usize) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    if char_idx >= chars.len() {
        return None;
    }
    for (start, end) in url_char_spans(line) {
        if char_idx >= start && char_idx < end {
            return Some(chars[start..end].iter().collect());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_https_url() {
        let line = "origin  https://github.com/rock117/Loom.git (fetch)";
        let idx = line.find("https").unwrap();
        let url = url_covering_char(line, idx + 5).unwrap();
        assert_eq!(url, "https://github.com/rock117/Loom.git");
        assert!(!url_char_spans(line).is_empty());
    }

    #[test]
    fn ignores_non_url() {
        let line = "hello world";
        assert!(url_covering_char(line, 3).is_none());
    }

    #[test]
    fn rejects_control_and_unknown_scheme() {
        assert!(!is_safe_open_url("https://evil\n.com"));
        assert!(!is_safe_open_url("javascript:alert(1)"));
        assert!(!is_safe_open_url(""));
        assert!(is_safe_open_url("https://github.com/rock117/Loom.git"));
    }
}
