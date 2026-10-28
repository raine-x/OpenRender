//! URL / media inspection helpers. No network access is ever performed.

/// Extract the lowercase scheme of a URL, if it is absolute.
///
/// Returns `None` for relative URLs, fragments and protocol-relative URLs.
pub fn scheme_of(url: &str) -> Option<String> {
    let url = url.trim();
    let mut bytes = url.bytes();
    match bytes.next() {
        Some(b) if b.is_ascii_alphabetic() => {}
        _ => return None,
    }
    for (i, b) in url.bytes().enumerate().skip(1) {
        match b {
            b':' => return Some(url[..i].to_ascii_lowercase()),
            b'/' | b'?' | b'#' => return None,
            b if b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.' => {}
            _ => return None,
        }
    }
    None
}

/// Extract the lowercase host of an absolute `scheme://authority/...` URL.
pub fn host_of(url: &str) -> Option<String> {
    let url = url.trim();
    let (_, rest) = url.split_once("://")?;
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .rsplit('@')
        .next()
        .unwrap_or_default();
    if authority.is_empty() {
        return None;
    }
    let host = if let Some(rest) = authority.strip_prefix('[') {
        // IPv6 literal: [::1]:443
        rest.split(']').next().unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

/// True when the URL carries control characters (never valid in a link).
pub fn has_control_chars(url: &str) -> bool {
    url.chars().any(|c| c.is_control())
}

/// Lowercase file extension of a URL path, without the dot.
pub fn extension_of(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name.rsplit_once('.')?.1;
    if ext.is_empty() {
        None
    } else {
        Some(ext.to_ascii_lowercase())
    }
}

/// Parse an image size hint from a markdown title: `=300x200`, `300x200`,
/// `300` (width only). Returns the dimensions plus the remaining caption.
pub fn parse_size_hint(title: &str) -> (Option<(u32, u32)>, String) {
    let raw = title.trim();
    let spec = raw.strip_prefix('=').unwrap_or(raw).trim();
    if spec.is_empty() {
        return (None, raw.to_string());
    }
    let mut parts = spec.split(['x', 'X', '*']);
    let width = parts.next().and_then(parse_dim);
    let height = parts.next().and_then(parse_dim);
    if parts.next().is_some() {
        return (None, raw.to_string());
    }
    match (width, height) {
        (Some(w), Some(h)) => (Some((w, h)), String::new()),
        (Some(w), None) if w > 0 => (Some((w, 0)), String::new()),
        _ => (None, raw.to_string()),
    }
}

fn parse_dim(s: &str) -> Option<u32> {
    let s = s.trim();
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<u32>().ok().filter(|n| *n > 0 && *n <= 100_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemes() {
        assert_eq!(scheme_of("https://a/b"), Some("https".into()));
        assert_eq!(scheme_of("HTTPS://a/b"), Some("https".into()));
        assert_eq!(scheme_of("agent://resource/1"), Some("agent".into()));
        assert_eq!(scheme_of("/relative/path"), None);
        assert_eq!(scheme_of("#frag"), None);
        assert_eq!(scheme_of("//cdn.example/a.png"), None);
        assert_eq!(scheme_of("javascript:alert(1)"), Some("javascript".into()));
        assert_eq!(scheme_of("1bad:):"), None);
    }

    #[test]
    fn hosts() {
        assert_eq!(
            host_of("https://www.Example.com:8443/x"),
            Some("www.example.com".into())
        );
        assert_eq!(host_of("https://user:pw@a.test/x"), Some("a.test".into()));
        assert_eq!(host_of("https://[::1]:80/x"), Some("::1".into()));
        assert_eq!(host_of("file:///sdcard/a.png"), None);
    }

    #[test]
    fn audio_detection() {
        assert_eq!(extension_of("https://x.test/a.mp3"), Some("mp3".into()));
        assert_eq!(extension_of("https://x.test/a.wav?x=1"), Some("wav".into()));
        assert_eq!(extension_of("https://x.test/a.PNG"), Some("png".into()));
        assert_eq!(extension_of("https://x.test/a"), None);
    }

    #[test]
    fn size_hints() {
        assert_eq!(
            parse_size_hint("=300x200"),
            (Some((300, 200)), String::new())
        );
        assert_eq!(parse_size_hint("300"), (Some((300, 0)), String::new()));
        assert_eq!(parse_size_hint("caption"), (None, "caption".into()));
        assert_eq!(parse_size_hint("=12x34x56"), (None, "=12x34x56".into()));
    }
}
