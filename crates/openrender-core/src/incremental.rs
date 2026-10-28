//! Incremental parsing primitives: block boundary detection and
//! fault-tolerant completion of unterminated inline constructs.
//!
//! Boundary detection is intentionally conservative. A commit point is only
//! chosen where later input cannot retroactively change the meaning of the
//! prefix:
//!
//! * a blank line outside a fenced code block,
//! * a closing code fence,
//! * an ATX heading line,
//! * a thematic break.
//!
//! Paragraph lines are *not* commit points because a following `---` turns
//! them into a setext heading, and a following table delimiter turns them into
//! a table header. They stay in the tail until a blank line arrives, or until
//! [`Config::max_tail_bytes`](crate::Config::max_tail_bytes) forces a split.

use std::borrow::Cow;

/// An opening fenced-code marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FenceOpen {
    pub ch: u8,
    pub len: usize,
}

/// Parse a fence opening line (``` or ~~~, up to 3 spaces of indentation).
pub fn fence_open(line: &str) -> Option<FenceOpen> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let indent = indent_of(line);
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let ch = *rest.as_bytes().first()?;
    if ch != b'`' && ch != b'~' {
        return None;
    }
    let len = run_len(rest.as_bytes(), 0, ch);
    if len < 3 {
        return None;
    }
    if ch == b'`' && rest[len..].contains('`') {
        return None;
    }
    Some(FenceOpen { ch, len })
}

/// Whether `line` closes a fence opened with `ch` repeated `len` times.
pub fn fence_close(line: &str, ch: u8, len: usize) -> bool {
    let line = line.trim();
    let count = line.bytes().take_while(|b| *b == ch).count();
    if count < len {
        return false;
    }
    line.as_bytes()[count..]
        .iter()
        .all(|b| *b == b' ' || *b == b'\t')
}

/// Byte offset just past the last safe block boundary in `text`, if any.
pub fn find_commit_point(text: &str) -> Option<usize> {
    let mut pos = 0usize;
    let mut fence: Option<FenceOpen> = None;
    let mut best: Option<usize> = None;

    while let Some(off) = text[pos..].find('\n') {
        let line_end = pos + off;
        let raw = &text[pos..line_end];
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let next = line_end + 1;

        match fence {
            Some(open) => {
                if fence_close(line, open.ch, open.len) {
                    fence = None;
                    best = Some(next);
                }
            }
            None => {
                if let Some(open) = fence_open(line) {
                    fence = Some(open);
                } else if line.trim().is_empty() {
                    best = Some(next);
                } else if is_atx_heading(line) || is_thematic_break(line) {
                    best = Some(next);
                }
            }
        }
        pos = next;
    }

    // A closing fence counts even without a trailing newline at EOF.
    if best.is_none() {
        if let Some(open) = fence {
            let last = text.rsplit('\n').next().unwrap_or("");
            if fence_close(last, open.ch, open.len) {
                best = Some(text.len());
            }
        }
    }
    best
}

/// The last fence marker that is still open at the end of `text`.
pub fn find_open_fence(text: &str) -> Option<FenceOpen> {
    let mut pos = 0usize;
    let mut fence: Option<FenceOpen> = None;
    while let Some(off) = text[pos..].find('\n') {
        let line_end = pos + off;
        let raw = &text[pos..line_end];
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        match fence {
            Some(open) => {
                if fence_close(line, open.ch, open.len) {
                    fence = None;
                }
            }
            None => {
                if let Some(open) = fence_open(line) {
                    fence = Some(open);
                }
            }
        }
        pos = line_end + 1;
    }
    if let Some(open) = fence {
        let last = text[pos..].strip_suffix('\r').unwrap_or(&text[pos..]);
        if fence_close(last, open.ch, open.len) {
            return None;
        }
    }
    fence
}

/// Whether `text` ends inside an unterminated fenced code block.
pub fn ends_in_open_fence(text: &str) -> bool {
    find_open_fence(text).is_some()
}

/// Append the minimal set of closing markers so an in-progress tail renders as
/// if it were finished: closes code fences, code spans, inline math, bold,
/// italic and strikethrough.
///
/// This is what makes arbitrary mid-stream prefixes renderable.
pub fn balance_inline(text: &str) -> Cow<'_, str> {
    if text.is_empty() {
        return Cow::Borrowed(text);
    }

    if let Some(fence) = find_open_fence(text) {
        let mut out = String::with_capacity(text.len() + fence.len + 2);
        out.push_str(text);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        for _ in 0..fence.len {
            out.push(fence.ch as char);
        }
        out.push('\n');
        return Cow::Owned(out);
    }

    let bytes = text.as_bytes();
    let mut code_open: Option<usize> = None;
    let mut math_open = false;
    let mut emph: Vec<(u8, usize)> = Vec::new();
    let mut i = 0usize;

    while i < bytes.len() {
        let b = bytes[i];

        if b == b'\\' {
            i += 2;
            continue;
        }

        if let Some(open) = code_open {
            if b == b'`' {
                let n = run_len(bytes, i, b'`');
                if n == open {
                    code_open = None;
                }
                i += n;
            } else {
                i += 1;
            }
            continue;
        }

        if math_open {
            let prev_is_space = i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b'\r');
            if b == b'$' && !prev_is_space {
                math_open = false;
            }
            i += 1;
            continue;
        }

        match b {
            b'`' => {
                let n = run_len(bytes, i, b'`');
                code_open = Some(n);
                i += n;
            }
            b'$' => {
                let next_is_text = text[i + 1..]
                    .chars()
                    .next()
                    .map(|c| !c.is_whitespace())
                    .unwrap_or(false);
                if next_is_text {
                    math_open = true;
                }
                i += 1;
            }
            b'*' | b'_' | b'~' => {
                let n = run_len(bytes, i, b);
                if b == b'~' && n < 2 {
                    i += n;
                    continue;
                }
                let prev = text[..i].chars().next_back();
                let next = text[i + n..].chars().next();
                let opening = is_opening(prev, next, n, b);
                let closing = is_closing(prev, next, n, b);
                if closing && emph.last().map(|(c, _)| *c == b).unwrap_or(false) {
                    emph.pop();
                } else if opening {
                    emph.push((b, n));
                }
                i += n;
            }
            _ => {
                i += 1;
            }
        }
    }

    if code_open.is_none() && !math_open && emph.is_empty() {
        return Cow::Borrowed(text);
    }

    let mut out = String::with_capacity(text.len() + 8);
    out.push_str(text);
    if let Some(n) = code_open {
        for _ in 0..n {
            out.push('`');
        }
    }
    if math_open {
        out.push('$');
    }
    for (ch, n) in emph.iter().rev() {
        for _ in 0..*n {
            out.push(*ch as char);
        }
    }
    Cow::Owned(out)
}

fn is_opening(prev: Option<char>, next: Option<char>, _n: usize, ch: u8) -> bool {
    let next_ok = next.map(|c| !c.is_whitespace()).unwrap_or(false);
    if !next_ok {
        return false;
    }
    if ch == b'_' {
        prev.map(|c| !c.is_alphanumeric()).unwrap_or(true)
    } else {
        true
    }
}

fn is_closing(prev: Option<char>, next: Option<char>, _n: usize, ch: u8) -> bool {
    let prev_ok = prev.map(|c| !c.is_whitespace()).unwrap_or(false);
    if !prev_ok {
        return false;
    }
    if ch == b'_' {
        next.map(|c| !c.is_alphanumeric()).unwrap_or(true)
    } else {
        true
    }
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn run_len(bytes: &[u8], start: usize, ch: u8) -> usize {
    bytes[start..].iter().take_while(|b| **b == ch).count()
}

fn is_atx_heading(line: &str) -> bool {
    if indent_of(line) > 3 {
        return false;
    }
    let rest = line.trim_start_matches(' ');
    let hashes = rest.bytes().take_while(|b| *b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return false;
    }
    let after = &rest[hashes..];
    after.is_empty() || after.starts_with(' ') || after.starts_with('\t')
}

fn is_thematic_break(line: &str) -> bool {
    if indent_of(line) > 3 {
        return false;
    }
    let mut marker = 0u8;
    let mut count = 0usize;
    for b in line.trim().bytes() {
        match b {
            b' ' | b'\t' => continue,
            b'*' | b'-' | b'_' => {
                if marker == 0 {
                    marker = b;
                } else if marker != b {
                    return false;
                }
                count += 1;
            }
            _ => return false,
        }
    }
    count >= 3
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_points() {
        assert_eq!(find_commit_point("hello\n"), None);
        assert_eq!(find_commit_point("hello\n\nworld"), Some(7));
        assert_eq!(find_commit_point("# title\nbody"), Some(8));
        assert_eq!(find_commit_point("a\n---\n"), Some(6));
        assert_eq!(find_commit_point("```\ncode\n```\n"), Some(13));
        assert_eq!(find_commit_point("```\ncode\n"), None);
    }

    #[test]
    fn open_fences() {
        assert_eq!(
            find_open_fence("```rust\nx"),
            Some(FenceOpen { ch: b'`', len: 3 })
        );
        assert_eq!(find_open_fence("~~~\nx\n~~~\n"), None);
        assert!(ends_in_open_fence("```\nx"));
        assert!(!ends_in_open_fence("```\nx\n```"));
    }

    #[test]
    fn balancing() {
        assert_eq!(balance_inline("**bold"), "**bold**");
        assert_eq!(balance_inline("*it"), "*it*");
        assert_eq!(balance_inline("`code"), "`code`");
        assert_eq!(balance_inline("$x^2"), "$x^2$");
        assert_eq!(balance_inline("```\nx"), "```\nx\n```\n");
        assert_eq!(balance_inline("plain"), "plain");
        assert_eq!(balance_inline("snake_case_name"), "snake_case_name");
        assert_eq!(balance_inline("**bold**"), "**bold**");
        assert_eq!(balance_inline(r"\*not emphasis"), r"\*not emphasis");
    }
}
