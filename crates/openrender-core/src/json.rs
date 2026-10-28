//! Hand-rolled JSON serialization of the IR.
//!
//! `serde` is deliberately avoided: the engine only ever *writes* JSON, so a
//! tiny specialized encoder keeps the binary small and is trivially fast.

use crate::ir::{Block, BlockKind, Delta, ListMarker, MathImage, Op, Span, SpanKind, Table};

pub fn encode(delta: &Delta) -> String {
    // ~180 bytes per op in practice; avoiding growth copies matters when a
    // host serializes many small deltas per second.
    let mut out = String::with_capacity(delta.ops.len() * 192 + 64);
    out.push_str("{\"v\":1,\"rev\":");
    write_u64(&mut out, delta.rev);
    out.push_str(",\"ops\":[");
    for (i, op) in delta.ops.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_op(&mut out, op);
    }
    out.push_str("]}");
    out
}

fn write_op(out: &mut String, op: &Op) {
    match op {
        Op::Put(block) => {
            out.push_str("{\"op\":\"put\",\"block\":");
            write_block(out, block);
            out.push('}');
        }
        Op::Append { id, text } => {
            out.push_str("{\"op\":\"append\",\"id\":");
            write_u64(out, *id);
            out.push_str(",\"text\":");
            write_string(out, text);
            out.push('}');
        }
        Op::Close { id } => {
            out.push_str("{\"op\":\"close\",\"id\":");
            write_u64(out, *id);
            out.push('}');
        }
        Op::Remove { id } => {
            out.push_str("{\"op\":\"remove\",\"id\":");
            write_u64(out, *id);
            out.push('}');
        }
    }
}

fn write_block(out: &mut String, block: &Block) {
    out.push_str("{\"id\":");
    write_u64(out, block.id);
    out.push_str(",\"type\":\"");
    out.push_str(block.kind.as_str());
    out.push('"');

    if block.level > 0 {
        out.push_str(",\"level\":");
        write_u64(out, block.level as u64);
    }
    if block.depth > 0 {
        out.push_str(",\"depth\":");
        write_u64(out, block.depth as u64);
    }
    if block.quote > 0 {
        out.push_str(",\"quote\":");
        write_u64(out, block.quote as u64);
    }
    if let Some(marker) = block.marker {
        out.push_str(",\"marker\":");
        match marker {
            ListMarker::Bullet => out.push_str("{\"ordered\":false}"),
            ListMarker::Ordered(n) => {
                out.push_str("{\"ordered\":true,\"number\":");
                write_u64(out, n);
                out.push('}');
            }
        }
    }
    if let Some(checked) = block.checked {
        out.push_str(if checked {
            ",\"checked\":true"
        } else {
            ",\"checked\":false"
        });
    }
    if block.kind == BlockKind::CodeBlock && !block.closed {
        out.push_str(",\"closed\":false");
    }
    if !block.alert.is_empty() {
        out.push_str(",\"alert\":");
        write_string(out, &block.alert);
    }
    if !block.name.is_empty() {
        out.push_str(",\"name\":");
        write_string(out, &block.name);
    }
    if !block.lang.is_empty() {
        out.push_str(",\"lang\":");
        write_string(out, &block.lang);
    }
    if !block.text.is_empty() {
        out.push_str(",\"text\":");
        write_string(out, &block.text);
    }
    if !block.url.is_empty() {
        out.push_str(",\"url\":");
        write_string(out, &block.url);
    }
    if !block.title.is_empty() {
        out.push_str(",\"title\":");
        write_string(out, &block.title);
    }
    if !block.description.is_empty() {
        out.push_str(",\"description\":");
        write_string(out, &block.description);
    }
    if let Some(math) = block.math.as_ref() {
        out.push_str(",\"math\":");
        write_math(out, math);
    }
    if let Some(table) = block.table.as_ref() {
        out.push_str(",\"table\":");
        write_table(out, table);
    }
    if !block.spans.is_empty() {
        out.push_str(",\"spans\":[");
        for (i, span) in block.spans.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_span(out, span);
        }
        out.push(']');
    }
    out.push('}');
}

fn write_span(out: &mut String, span: &Span) {
    out.push_str("{\"type\":\"");
    out.push_str(span.kind.as_str());
    out.push('"');

    match span.kind {
        SpanKind::Text | SpanKind::Code => {
            out.push_str(",\"text\":");
            write_string(out, &span.text);
            write_styles(out, span);
            if !span.url.is_empty() {
                out.push_str(",\"url\":");
                write_string(out, &span.url);
            }
            if !span.title.is_empty() {
                out.push_str(",\"title\":");
                write_string(out, &span.title);
            }
        }
        SpanKind::Image | SpanKind::Audio => {
            out.push_str(",\"src\":");
            write_string(out, &span.url);
            if !span.text.is_empty() {
                out.push_str(",\"alt\":");
                write_string(out, &span.text);
            }
            if !span.title.is_empty() {
                out.push_str(",\"title\":");
                write_string(out, &span.title);
            }
            if span.width > 0 {
                out.push_str(",\"width\":");
                write_u64(out, span.width as u64);
            }
            if span.height > 0 {
                out.push_str(",\"height\":");
                write_u64(out, span.height as u64);
            }
        }
        SpanKind::LatexInline => {
            out.push_str(",\"tex\":");
            write_string(out, &span.text);
            if let Some(math) = span.math.as_ref() {
                out.push_str(",\"math\":");
                write_math(out, math);
            }
        }
        SpanKind::FootnoteRef => {
            out.push_str(",\"name\":");
            write_string(out, &span.name);
        }
        SpanKind::SoftBreak | SpanKind::HardBreak => {}
    }
    out.push('}');
}

fn write_styles(out: &mut String, span: &Span) {
    if span.bold {
        out.push_str(",\"bold\":true");
    }
    if span.italic {
        out.push_str(",\"italic\":true");
    }
    if span.strike {
        out.push_str(",\"strike\":true");
    }
}

fn write_table(out: &mut String, table: &Table) {
    out.push_str("{\"align\":[");
    for (i, align) in table.align.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_string(out, align);
    }
    out.push_str("],\"header\":[");
    write_cells(out, &table.header);
    out.push_str("],\"rows\":[");
    for (i, row) in table.rows.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('[');
        write_cells(out, row);
        out.push(']');
    }
    out.push_str("]}");
}

fn write_cells(out: &mut String, cells: &[Vec<Span>]) {
    for (i, cell) in cells.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('[');
        for (j, span) in cell.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            write_span(out, span);
        }
        out.push(']');
    }
}

fn write_math(out: &mut String, math: &MathImage) {
    out.push_str("{\"mime\":");
    write_string(out, &math.mime);
    out.push_str(",\"width\":");
    write_u64(out, math.width as u64);
    out.push_str(",\"height\":");
    write_u64(out, math.height as u64);
    out.push_str(",\"data\":");
    write_string(out, &base64(&math.data));
    out.push('}');
}

fn write_u64(out: &mut String, value: u64) {
    let mut buf = [0u8; 20];
    let mut n = value;
    let mut i = buf.len();
    if n == 0 {
        out.push('0');
        return;
    }
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    out.push_str(std::str::from_utf8(&buf[i..]).unwrap_or("0"));
}

fn write_string(out: &mut String, value: &str) {
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                out.push_str("\\u");
                const HEX: &[u8; 16] = b"0123456789abcdef";
                let n = c as u32;
                for shift in [12, 8, 4, 0] {
                    out.push(HEX[((n >> shift) & 0xf) as usize] as char);
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping() {
        let mut out = String::new();
        write_string(&mut out, "a\"b\\c\nd\u{1}");
        assert_eq!(out, "\"a\\\"b\\\\c\\nd\\u0001\"");
    }

    #[test]
    fn base64_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
    }
}
