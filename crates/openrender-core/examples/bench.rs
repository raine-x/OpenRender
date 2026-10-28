//! Rough throughput benchmark (no external harness).
//!
//! Run with: `cargo run --release -p openrender-core --example bench`

use std::time::Instant;

use openrender::{Config, Engine};

const DENSE: &str = "## Section\n\nSome **bold** and *italic* text with `code` and a [link](https://example.com).\n\n- item one\n- item two\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n> quoted paragraph\n\n";

const PROSE: &str = "## A natural paragraph\n\nThe quick brown fox jumps over the lazy dog while a streaming markdown engine \
keeps parsing only the unstable tail. Settled blocks are cached once and re-emitted only when they change, \
which keeps the cost of every `feed` call proportional to the newly appended text rather than to the whole \
document. This paragraph is deliberately long so the block density resembles real chat content.\n\n\
A second paragraph with **emphasis**, a [link](https://example.org/docs) and inline `code` follows. \
Lists appear occasionally:\n\n- first item\n- second item\n- third item with **bold**\n\n";

struct Corpus {
    name: &'static str,
    doc: String,
}

fn build(name: &'static str, unit: &str) -> Corpus {
    let mut doc = String::with_capacity(1_100_000);
    while doc.len() < 1_000_000 {
        doc.push_str(unit);
    }
    Corpus { name, doc }
}

fn main() {
    let cfg = Config::default();
    for corpus in [build("dense (36k blocks/MB)", DENSE), build("prose", PROSE)] {
        println!("== {} == document: {} bytes", corpus.name, corpus.doc.len());

        for run in 0..2 {
            let start = Instant::now();
            let delta = Engine::render_once(&corpus.doc, &cfg);
            let render = start.elapsed();
            let start = Instant::now();
            let json = delta.to_json();
            let serialize = start.elapsed();
            println!(
                "  one-shot #{run}: render {:?}, json {:?} ({} blocks, {} JSON bytes, {:.1} MB/s parse)",
                render,
                serialize,
                delta.ops.len(),
                json.len(),
                corpus.doc.len() as f64 / 1_000_000.0 / render.as_secs_f64()
            );
        }

        let mut engine = Engine::new(Config {
            throttle_ms: 0,
            ..cfg.clone()
        });
        let start = Instant::now();
        let mut json_bytes = 0usize;
        let mut bytes = 0usize;
        while bytes < corpus.doc.len() {
            let end = (bytes + 4096).min(corpus.doc.len());
            let end = floor_boundary(&corpus.doc, end);
            let delta = engine.feed(&corpus.doc[bytes..end]);
            json_bytes += delta.to_json().len();
            bytes = end;
        }
        let delta = engine.finish();
        json_bytes += delta.to_json().len();
        let elapsed = start.elapsed();
        println!(
            "  streaming 4 KiB chunks: {:?} ({} ops at finish, {} JSON bytes, {:.1} MB/s, tail {} B)",
            elapsed,
            delta.ops.len(),
            json_bytes,
            corpus.doc.len() as f64 / 1_000_000.0 / elapsed.as_secs_f64(),
            engine.tail_bytes()
        );
    }
}

fn floor_boundary(text: &str, mut index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}
