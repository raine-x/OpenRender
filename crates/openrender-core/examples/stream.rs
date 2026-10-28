//! Streaming API example.
//!
//! Run with: `cargo run -p openrender-core --example stream`

use openrender::{Config, Engine};

fn main() {
    let mut engine = Engine::new(Config {
        throttle_ms: 0,
        ..Config::default()
    });

    let chunks = [
        "# Streaming demo\n\n",
        "This paragraph is **bold so far",
        "** and keeps going.\n\n",
        "```rust\n",
        "fn main() {\n",
        "    println!(\"hi\");\n",
        "}\n",
        "```\n\n",
        "![voice](https://example.com/voice.mp3)\n\n",
    ];

    for (i, chunk) in chunks.iter().enumerate() {
        let delta = engine.feed(chunk);
        println!("feed {i}: {}", delta.to_json());
    }

    let done = engine.finish();
    println!("finish: {}", done.to_json());
    println!("revision: {}", engine.revision());
}
