//! Streaming-first Markdown engine.
//!
//! Feed UTF-8 text incrementally; receive language-neutral IR deltas that a
//! Kotlin, Swift, C or Rust host can render directly. No WebView, no
//! JavaScript runtime, no GUI dependency.
//!
//! ```no_run
//! use openrender::{Config, Engine};
//!
//! let mut engine = Engine::new(Config::default());
//! let delta = engine.feed("# Hello\n\nwor");
//! println!("{}", delta.to_json());
//! let delta = engine.feed("ld\n");
//! println!("{}", delta.to_json());
//! let done = engine.finish();
//! println!("{}", done.to_json());
//! ```

pub mod config;
pub mod engine;
pub mod ffi;
pub mod ir;

mod incremental;
mod json;
mod markdown;
mod sanitize;

#[cfg(feature = "jni")]
mod jni_bridge;

pub use config::Config;
pub use engine::Engine;
pub use ir::{Block, BlockKind, Delta, ListMarker, MathImage, Op, Span, SpanKind, Table};
