//! The streaming engine: append-only input, incremental output.
//!
//! # Pipeline
//!
//! ```text
//! feed(text) ──► pending (throttle buffer)
//!                    │ flush
//!                    ▼
//!                held (unstable tail)
//!                    │ find commit point (blank line / closed fence / heading / rule)
//!                    ▼
//!             committed blocks ──► rendered once ──► Delta { put }
//!                    │
//!                    └─ remaining tail re-rendered each flush ──► Delta { put }
//! ```
//!
//! Three properties keep this cheap:
//!
//! 1. Settled blocks are parsed **once**; only the tail is ever re-parsed.
//! 2. The tail is bounded by `max_tail_bytes`; oversized tails are force-split
//!    at a line boundary, and huge code fences are streamed line-by-line with
//!    `append` ops instead of being re-emitted.
//! 3. High-frequency `feed` calls are coalesced by `throttle_ms`.
//!
//! Together they give per-call cost proportional to the newly appended text.

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use crate::config::Config;
use crate::incremental::{
    balance_inline, ends_in_open_fence, fence_close, find_commit_point, find_open_fence,
};
use crate::ir::{Block, BlockKind, Delta, Op};
use crate::markdown::{self, RenderArgs};

/// A huge code fence that is being streamed line-by-line. The block itself is
/// committed; new complete lines are appended with `Op::Append`.
struct Spill {
    id: u64,
    ch: u8,
    len: usize,
    partial: String,
}

pub struct Engine {
    cfg: Config,
    pending: String,
    held: String,
    committed: u64,
    live: u64,
    rev: u64,
    total_bytes: usize,
    finished: bool,
    /// Monotonic clock for throttling. Not available on `wasm32`; wasm hosts
    /// should use `feed_at` (or `throttle_ms = 0`).
    #[cfg(not(target_arch = "wasm32"))]
    started: Instant,
    last_parse_ms: u64,
    spill: Option<Spill>,
    #[cfg(feature = "latex")]
    math: Option<std::sync::Arc<dyn openrender_latex::MathRenderer>>,
}

impl Engine {
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            pending: String::new(),
            held: String::new(),
            committed: 0,
            live: 0,
            rev: 0,
            total_bytes: 0,
            finished: false,
            #[cfg(not(target_arch = "wasm32"))]
            started: Instant::now(),
            last_parse_ms: 0,
            spill: None,
            #[cfg(feature = "latex")]
            math: None,
        }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.cfg
    }

    /// Last emitted revision. Increments only when a delta carries operations.
    pub fn revision(&self) -> u64 {
        self.rev
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    /// Bytes currently held as unstable tail (for diagnostics/benchmarks).
    pub fn tail_bytes(&self) -> usize {
        self.held.len()
    }

    #[cfg(feature = "latex")]
    pub fn set_math_renderer(
        &mut self,
        renderer: Option<std::sync::Arc<dyn openrender_latex::MathRenderer>>,
    ) {
        self.math = renderer;
    }

    /// Install the built-in RaTeX renderer (feature `ratex`).
    #[cfg(feature = "ratex")]
    pub fn use_ratex(&mut self) {
        self.math = Some(std::sync::Arc::new(
            openrender_latex::RatexRenderer::default(),
        ));
    }

    fn now_ms(&self) -> u64 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.started.elapsed().as_millis() as u64
        }
        #[cfg(target_arch = "wasm32")]
        {
            0
        }
    }

    /// Append text, returning a delta when the throttle window allows parsing.
    ///
    /// Calls that arrive inside the throttle window are buffered; the delta is
    /// empty and the text is parsed on the next `feed`/`flush`/`finish`.
    pub fn feed(&mut self, text: &str) -> Delta {
        let now = self.now_ms();
        self.feed_at(text, now)
    }

    /// Deterministic variant of [`Engine::feed`] for tests and custom clocks.
    pub fn feed_at(&mut self, text: &str, now_ms: u64) -> Delta {
        let mut delta = Delta {
            rev: self.rev,
            ops: Vec::new(),
        };
        if self.finished {
            return delta;
        }
        if !text.is_empty() {
            self.append_limited(text);
        }
        let due = self.cfg.throttle_ms == 0
            || now_ms.saturating_sub(self.last_parse_ms) >= self.cfg.throttle_ms as u64;
        if due {
            self.process(now_ms, &mut delta);
        }
        delta
    }

    /// Force a parse of any buffered text.
    pub fn flush(&mut self) -> Delta {
        let now = self.now_ms();
        self.flush_at(now)
    }

    pub fn flush_at(&mut self, now_ms: u64) -> Delta {
        let mut delta = Delta {
            rev: self.rev,
            ops: Vec::new(),
        };
        self.process(now_ms, &mut delta);
        delta
    }

    /// Commit the remaining tail (auto-closing open constructs) and end the
    /// document. The engine can be [`Engine::reset`] and reused afterwards.
    pub fn finish(&mut self) -> Delta {
        let mut delta = Delta {
            rev: self.rev,
            ops: Vec::new(),
        };
        if self.finished {
            return delta;
        }
        self.finished = true;
        self.last_parse_ms = self.now_ms();

        if !self.pending.is_empty() {
            self.held.push_str(&self.pending);
            self.pending.clear();
        }

        if let Some(mut spill) = self.spill.take() {
            if !spill.partial.is_empty() {
                delta.ops.push(Op::Append {
                    id: spill.id,
                    text: std::mem::take(&mut spill.partial),
                });
            }
            delta.ops.push(Op::Close { id: spill.id });
        }

        if !self.held.is_empty() {
            let unclosed = ends_in_open_fence(&self.held);
            let balanced = balance_inline(&self.held);
            let blocks = markdown::render(balanced.as_ref(), &self.render_args(unclosed));
            self.held.clear();
            self.push_committed(blocks, &mut delta);
        }

        let live = self.committed;
        self.sync_live(live, &mut delta);
        if !delta.ops.is_empty() {
            self.rev += 1;
        }
        delta.rev = self.rev;
        delta
    }

    pub fn reset(&mut self) {
        self.pending.clear();
        self.held.clear();
        self.committed = 0;
        self.live = 0;
        self.rev = 0;
        self.total_bytes = 0;
        self.finished = false;
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.started = Instant::now();
        }
        self.last_parse_ms = 0;
        self.spill = None;
    }

    /// One-shot rendering convenience: no throttling, everything committed.
    pub fn render_once(text: &str, cfg: &Config) -> Delta {
        let cap = text.len().min(cfg.max_document_bytes);
        let cap = floor_char_boundary(text, cap);
        let input = &text[..cap];

        let unclosed = ends_in_open_fence(input);
        let balanced = balance_inline(input);
        let args = RenderArgs {
            cfg,
            unclosed_fence: unclosed,
            #[cfg(feature = "latex")]
            math: None,
        };
        let blocks = markdown::render(balanced.as_ref(), &args);

        let mut delta = Delta {
            rev: 1,
            ops: Vec::with_capacity(blocks.len()),
        };
        for (i, mut block) in blocks.into_iter().enumerate() {
            block.id = i as u64;
            delta.ops.push(Op::Put(block));
        }
        delta
    }

    fn render_args(&self, unclosed: bool) -> RenderArgs<'_> {
        RenderArgs {
            cfg: &self.cfg,
            unclosed_fence: unclosed,
            #[cfg(feature = "latex")]
            math: self.math.as_deref(),
        }
    }

    fn append_limited(&mut self, text: &str) {
        if self.cfg.max_blocks > 0 && self.committed >= self.cfg.max_blocks {
            return;
        }
        let room = self.cfg.max_document_bytes.saturating_sub(self.total_bytes);
        if room == 0 {
            return;
        }
        let take = text.len().min(room);
        let take = floor_char_boundary(text, take);
        self.pending.push_str(&text[..take]);
        self.total_bytes += take;
    }

    fn process(&mut self, now_ms: u64, delta: &mut Delta) {
        if self.pending.is_empty() {
            return;
        }
        self.last_parse_ms = now_ms;
        self.held.push_str(&self.pending);
        self.pending.clear();

        self.run_incremental(delta);

        if !delta.ops.is_empty() {
            self.rev += 1;
        }
        delta.rev = self.rev;
    }

    fn run_incremental(&mut self, delta: &mut Delta) {
        if self.spill.is_some() && !self.advance_spill(delta) {
            return;
        }

        if let Some(cut) = find_commit_point(&self.held) {
            if cut > 0 {
                self.commit_prefix(cut, delta);
            }
        }

        self.maybe_spill_or_split(delta);
        self.render_tail(delta);
    }

    fn commit_prefix(&mut self, cut: usize, delta: &mut Delta) {
        let cut = cut.min(self.held.len());
        let cut = floor_char_boundary(&self.held, cut);
        if cut == 0 {
            return;
        }
        let chunk: String = self.held.drain(..cut).collect();
        let unclosed = ends_in_open_fence(&chunk);
        let blocks = markdown::render(&chunk, &self.render_args(unclosed));
        self.push_committed(blocks, delta);
    }

    fn push_committed(&mut self, blocks: Vec<Block>, delta: &mut Delta) {
        let start = self.committed;
        let count = blocks.len() as u64;
        for (i, mut block) in blocks.into_iter().enumerate() {
            block.id = start + i as u64;
            delta.ops.push(Op::Put(block));
        }
        self.committed = start + count;
    }

    fn maybe_spill_or_split(&mut self, delta: &mut Delta) {
        if self.held.len() <= self.cfg.max_tail_bytes {
            return;
        }

        if let Some(fence) = find_open_fence(&self.held) {
            let Some(nl) = self.held.rfind('\n') else {
                return;
            };
            if nl == 0 {
                return;
            }
            let cut = nl + 1;
            self.commit_prefix(cut, delta);

            let last_id = self.committed.checked_sub(1);
            let last_is_code = matches!(
                delta.ops.last(),
                Some(Op::Put(block)) if block.kind == BlockKind::CodeBlock
            );
            if let Some(last_id) = last_id {
                if last_is_code {
                    let partial = std::mem::take(&mut self.held);
                    self.spill = Some(Spill {
                        id: last_id,
                        ch: fence.ch,
                        len: fence.len,
                        partial,
                    });
                }
            }
            return;
        }

        if let Some(nl) = self.held.rfind('\n') {
            let cut = nl + 1;
            if cut > 0 {
                self.commit_prefix(cut, delta);
            }
            return;
        }

        // A single extremely long line: split near the tail cap at a word
        // boundary so per-feed work stays bounded. This intentionally splits
        // one paragraph into two blocks (documented trade-off).
        let target = self
            .cfg
            .max_tail_bytes
            .min(self.held.len().saturating_sub(1));
        let mut cut = floor_char_boundary(&self.held, target);
        if let Some(pos) = self.held[..cut].rfind(char::is_whitespace) {
            let word_end = pos
                + self.held[pos..]
                    .chars()
                    .next()
                    .map(|c| c.len_utf8())
                    .unwrap_or(1);
            cut = word_end;
        }
        if cut == 0 {
            cut = floor_char_boundary(&self.held, target.max(1));
        }
        if cut > 0 && cut < self.held.len() {
            self.commit_prefix(cut, delta);
        }
    }

    fn advance_spill(&mut self, delta: &mut Delta) -> bool {
        let Some(mut spill) = self.spill.take() else {
            return true;
        };

        let mut text = std::mem::take(&mut spill.partial);
        text.push_str(&self.held);
        self.held.clear();

        let mut pos = 0usize;
        let mut closed_at: Option<usize> = None;
        while let Some(off) = text[pos..].find('\n') {
            let line_end = pos + off;
            let raw = &text[pos..line_end];
            let line = raw.strip_suffix('\r').unwrap_or(raw);
            if fence_close(line, spill.ch, spill.len) {
                closed_at = Some(line_end + 1);
                break;
            }
            pos = line_end + 1;
        }

        if pos > 0 {
            delta.ops.push(Op::Append {
                id: spill.id,
                text: text[..pos].to_string(),
            });
        }

        match closed_at {
            Some(end) => {
                delta.ops.push(Op::Close { id: spill.id });
                self.held.push_str(&text[end..]);
                true
            }
            None => {
                spill.partial = text[pos..].to_string();
                self.spill = Some(spill);
                false
            }
        }
    }

    fn render_tail(&mut self, delta: &mut Delta) {
        if self.held.is_empty() {
            let live = self.committed;
            self.sync_live(live, delta);
            return;
        }

        let unclosed = ends_in_open_fence(&self.held);
        let balanced = balance_inline(&self.held);
        let blocks = markdown::render(balanced.as_ref(), &self.render_args(unclosed));

        let start = self.committed;
        let count = blocks.len() as u64;
        for (i, mut block) in blocks.into_iter().enumerate() {
            block.id = start + i as u64;
            delta.ops.push(Op::Put(block));
        }
        self.sync_live(start + count, delta);
    }

    fn sync_live(&mut self, live: u64, delta: &mut Delta) {
        if self.live > live {
            for id in live..self.live {
                delta.ops.push(Op::Remove { id });
            }
        }
        self.live = live;
    }
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> Engine {
        Engine::new(Config {
            throttle_ms: 0,
            ..Config::default()
        })
    }

    #[test]
    fn paragraph_streams_in_place() {
        let mut engine = engine();
        let first = engine.feed("Hello");
        assert!(first.to_json().contains("\"type\":\"paragraph\""));
        assert!(first.to_json().contains("Hello"));

        let second = engine.feed(" world\n\n");
        let json = second.to_json();
        assert!(json.contains("\"type\":\"paragraph\""));
        assert!(json.contains("Hello world"));
        assert!(engine.tail_bytes() == 0);
    }

    #[test]
    fn tail_is_bounded() {
        let mut engine = Engine::new(Config {
            throttle_ms: 0,
            max_tail_bytes: 512,
            ..Config::default()
        });
        for _ in 0..200 {
            engine.feed("word ");
        }
        assert!(engine.tail_bytes() <= 512 + 8);
    }

    #[test]
    fn huge_fence_is_appended_not_reemitted() {
        let mut engine = Engine::new(Config {
            throttle_ms: 0,
            max_tail_bytes: 256,
            ..Config::default()
        });
        engine.feed("```rust\n");
        let mut saw_append = false;
        let mut max_delta = 0usize;
        for _ in 0..200 {
            let delta = engine.feed("let x = 1;\n");
            let json = delta.to_json();
            max_delta = max_delta.max(json.len());
            if json.contains("\"op\":\"append\"") {
                saw_append = true;
            }
        }
        let eof = engine.finish().to_json();
        assert!(saw_append);
        assert!(eof.contains("\"op\":\"close\""));
        // Deltas stay proportional to appended text, never to the whole block.
        assert!(max_delta < 2048);
    }

    #[test]
    fn reset_reuses_engine() {
        let mut engine = engine();
        let _ = engine.feed("a\n\n");
        let _ = engine.finish();
        engine.reset();
        let delta = engine.feed("b\n\n");
        assert!(delta.to_json().contains("\"id\":0"));
    }
}
