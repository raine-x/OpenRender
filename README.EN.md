# OpenRender

**English** | [简体中文](README.ZH.md)

**A streaming-first Markdown engine in Rust.** Feed it text token by token; it emits language-neutral rendering directives with stable block ids. No WebView, no JavaScript runtime, no GUI toolkit — a native parser plus a host-owned renderer.

[![License: MIT](https://img.shields.io/badge/license-MIT-111827.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-111827.svg)](https://www.rust-lang.org)
[![Platforms](https://img.shields.io/badge/platforms-Android%20%7C%20Linux%20%7C%20macOS%20%7C%20Windows-111827.svg)](#build)
[![Status](https://img.shields.io/badge/status-0.1%20preview-111827.svg)](#roadmap)

![OpenRender IR preview](preview/preview.png)

*A real screenshot of the browser preview: 22 IR blocks produced by the Rust engine and rendered by a thin host-side renderer. Source: [`preview/index.html`](preview/index.html) · generator: [`examples/preview.rs`](crates/openrender-core/examples/preview.rs).*

---

## Why OpenRender

Streaming chat, live notes, AI output panes and terminal UIs all share the same problem: Markdown arrives in fragments, and the usual answer is to re-parse the whole document on every fragment. That is O(N²), it drops frames on mobile, and it forces hosts into a WebView that costs tens of megabytes, executes JavaScript, and cannot hand structured data back to native code.

OpenRender takes a different route:

1. **Commit only what cannot change.** Settled blocks are parsed once and cached.
2. **Re-parse only the tail.** Work per call is bounded by the new text plus a fixed tail window.
3. **Return directives, not pixels.** The host decides typography, media playback, cards and gestures.

The result is a ~500 KiB native library that keeps a 1 MB document streaming at interactive rates, with deterministic memory ceilings and a C ABI / JNI surface.

---

## Design principles

| Principle | Consequence |
|---|---|
| **Linear streaming** | Per-`feed` cost is O(new text + `max_tail_bytes`), never O(document). |
| **Bounded memory** | Tail cap, document cap, block cap; huge code fences stream via `append` instead of re-sending. |
| **Stable identity** | Every block carries a dense, monotonic `block_id`; the UI diffs by id. |
| **Host-owned rendering** | The engine never loads images, plays audio or paints text. It emits URLs, sizes and semantics. |
| **Safe by construction** | URL scheme allow-list, no HTML execution, panic-safe FFI, iterative parser (no stack recursion). |
| **Small surface** | One parsing dependency (`pulldown-cmark`); hand-written JSON writer; optional features only. |

---

## Architecture

```text
feed(text)
   │  append to throttle buffer (coalesce bursts)
   ▼
held tail  ──►  commit-point scan
   │               blank line · closed fence · ATX heading · thematic break
   ├── settled chunk ──► pulldown-cmark ──► IR blocks ──► delta { put }
   └── unstable tail ──► balance open markers ──► re-render ──► delta { put }
```

**Commit points are conservative.** A paragraph line is *not* a commit point: a following `---` would turn it into a setext heading, and a table delimiter would turn it into a table header. Only positions whose meaning cannot be rewritten by later input are committed.

**Oversized tails are split.** If the unstable tail exceeds `max_tail_bytes` (8 KiB by default), it is committed at a line/word boundary. This is the only case where one paragraph may become two blocks.

**Huge code fences spill.** Once an open fence grows past the cap, its complete lines are emitted as `append` operations against a committed code block; the closing fence emits `close`. Streaming stays proportional to the new lines rather than to the whole fence.

**Fault tolerance.** Before rendering the tail, unterminated fences, code spans, `$…$`, `**`, `*`, `_` and `~~` are closed with minimal markers. A truncated stream still renders as if it were finished (`closed: false` marks synthetic closure).

**Amortized complexity.** Each byte is parsed once when it settles and a bounded number of times while it is in the tail. Total cost is linear in document size; the constant is the tail cap.

---

## The IR in 30 seconds

One call returns one delta. Ops are applied by `block_id`:

```json
{
  "v": 1,
  "rev": 7,
  "ops": [
    { "op": "put",    "block": { "id": 3, "type": "paragraph", "spans": [
        { "type": "text", "text": "Hello " },
        { "type": "text", "text": "world", "bold": true },
        { "type": "latex_inline", "tex": "E=mc^2" }
    ]}},
    { "op": "append", "id": 4, "text": "more code\n" },
    { "op": "close",  "id": 4 },
    { "op": "remove", "id": 9 }
  ]
}
```

| Op | Meaning |
|---|---|
| `put` | Insert or replace a block. Re-sent while a block is still the unstable tail. |
| `append` | Append text to a streaming code block (large fences). |
| `close` | The code block received its closing fence. |
| `remove` | A tail block disappeared (the tail shrank). |

**Blocks:** `paragraph`, `heading` (`level`), `code_block` (`lang`, `text`, `closed`), `list_item` (`depth`, `marker`, `checked`), `table` (`align`/`header`/`rows`), `rule`, `footnote_def`, `latex_block` (`tex`, optional PNG), `audio` (`url`, `title`), `link_preview` (`url`, `title`, `description`).

**Spans:** `text` (`bold`/`italic`/`strike`/`url`/`title`), `code`, `image` (`src`/`alt`/`width`/`height`), `audio`, `latex_inline` (`tex`, optional PNG), `footnote_ref`, `softbreak`, `hardbreak`.

Styles are flattened (boolean flags) so a host can build an attributed string in a single pass. Ids are dense and 0-based in document order; settled ids never change.

---

## Supported Markdown

| Area | Support |
|---|---|
| CommonMark | headings, paragraphs, ordered/unordered/nested lists, block quotes, fenced and indented code, inline code, bold, italic, links, images, thematic breaks |
| GFM | tables with alignment, task lists, strikethrough, GFM alert blocks (`NOTE` / `TIP` / `IMPORTANT` / `WARNING` / `CAUTION`) |
| Extensions | footnotes, inline `$…$` and block `$$…$$` math, smart punctuation (optional) |
| Media | audio URLs (`mp3`, `wav`, `m4a`, `aac`, `ogg`, `opus`, `flac`, `weba`, `3gp`, `amr`, configurable) become `audio` blocks/spans |
| Links | custom schemes (e.g. `agent://resource/42`), size hints (`=640x360`), link preview cards for configured domains |
| LaTeX | TeX source always exported; optional RaTeX (pure Rust, KaTeX-compatible) renders PNG with embedded fonts |

HTML is never executed. By default it is preserved as literal text; `drop_html` removes it entirely.

---

## Quantified results

**Environment:** Intel Core i7-12700H (14C/20T), 16 GB RAM, Windows x86_64, Rust 1.99.1, `--release` with `opt-level = 3`, fat LTO, `codegen-units = 1`, `strip = "symbols"`. Medians of four one-shot samples and two streaming samples on an otherwise idle machine, measured with the repository's own harness:

```bash
cargo run --release -p openrender-core --example bench
```

Two synthetic 1 MB corpora:

- **Prose** — headings, long paragraphs, emphasis, links, inline code, occasional lists. 10,068 blocks. Closest to chat/LLM output.
- **Dense** — a repeated section with a table, quote and task list. 36,150 blocks per MB (~28 bytes per block). Adversarial stress case for block bookkeeping.

| Corpus | Render | JSON serialize | Streaming, 4 KiB feeds (incl. JSON) | Parse throughput |
|---|---:|---:|---:|---:|
| Prose (1 MB) | **12.1 ms** | **2.7 ms** | **13.8 ms** | 65–90 MB/s |
| Dense (1 MB) | **50.7 ms** | **7.3 ms** | **70 ms** | 14–22 MB/s |

- Streaming numbers include serializing every delta to JSON, i.e. the full path a C/JNI host pays.
- JSON sizes: 2.4 MB for the prose document, 6.5 MB for the dense one — an important host-side budget item.
- Peak working set for the whole benchmark process (both 1 MB corpora, block maps and JSON strings): **69.6 MiB**.
- `feed` work is O(new text + ≤ 8 KiB tail); `finish` renders the remaining tail once. There is no full-document re-parse on any call.
- ARM64 device numbers are **not** claimed here. Expect roughly 2–3× slower single-core performance than this laptop; the prose corpus has ample head-room for a 200 ms budget, the dense corpus is deliberately close to the limit.

### Binary size (Windows x64, release, stripped)

| Build | `openrender.dll` |
|---|---:|
| default (parser + IR + C ABI) | **494 KiB** |
| `--features jni` | **521 KiB** |
| `--features latex` | **499 KiB** |
| staticlib (`openrender.lib`, includes Rust `std`) | **15.5 MiB** |

`ratex` is opt-in and not included in the table: it embeds KaTeX fonts and is intended only for hosts that want offline PNG math. Android `.so` sizes will differ from Windows but stay in the same order of magnitude for the core build. The `wasm32-unknown-unknown` core used by the live demo is **301 KiB**.

---

## Configuration

Every knob is available from Rust and the C ABI; the Kotlin wrapper exposes the common subset (constructor + `configure()`):

| Setting | Default | Purpose |
|---|---|---|
| `throttle_ms` | `8` | Minimum interval between parses; `0` renders on arrival. |
| `max_tail_bytes` | `8192` | Caps per-feed re-parsing and forced-split behavior. |
| `max_document_bytes` | `16 MiB` | Hard input cap; extra text is dropped. |
| `max_blocks` | `1,000,000` | Hard block cap; `0` disables it. |
| `parser_flags` | `0x3F` | Tables, strikethrough, task lists, footnotes, GFM alerts, math. |
| `enable_previews` + `preview_domains` | off / empty | Turns standalone links into preview cards. |
| `soft_break_as_space` | off | Soft breaks become spaces instead of `softbreak` spans. |
| `drop_html` | off | Drop raw HTML instead of emitting literal text. |
| `enable_size_hints` | on | Interpret image titles like `=640x360`. |
| `audio_extensions` | mp3/wav/m4a/… | Extra extensions become audio directives. |
| `allowed_schemes` | http, https, file, mailto, tel | Everything else degrades to plain text. |

The engine is synchronous: it parses inside `feed`/`flush`/`finish` on the caller's thread. There is no timer and no background worker; a host that wants frame-rate output calls `flush()` from its render loop.

---

## Usage

### Rust

```rust
use openrender::{Config, Engine};

let mut engine = Engine::new(Config::default());
for chunk in ["# Title\n\n", "Streaming **bold", "** text.\n"] {
    println!("{}", engine.feed(chunk).to_json());
}
println!("{}", engine.finish().to_json());
```

One-shot rendering: `Engine::render_once(markdown, &config).to_json()`.

### C

```c
#include "engine.h"

OrEngine *engine = or_engine_new(NULL);
char *delta = or_engine_feed(engine, text, len);
or_string_free(delta);
char *done = or_engine_finish(engine);
or_string_free(done);
or_engine_free(engine);
```

Full example: [`examples/c/demo.c`](examples/c/demo.c). Header: [`include/engine.h`](include/engine.h) (generated by cbindgen).

### Kotlin / Android

```kotlin
val engine = NativeEngine()
val applier = DeltaApplier()

engine.feed("# Hello\n\nwor").let(applier::apply)
engine.feed("ld\n\n").let(applier::apply)
engine.finish().let(applier::apply)

adapter.submitList(applier.snapshot())
engine.close()
```

[`NativeEngine.kt`](android/app/src/main/java/ai/openrender/NativeEngine.kt) includes the JNI wrapper, `DeltaApplier` (`put`/`append`/`close`/`remove`) and parser flag constants.

### Optional math rendering

```c
// Host-supplied renderer over the C ABI:
or_engine_set_math_renderer(engine, render_math, free_math, NULL);

// Or the built-in pure-Rust RaTeX engine (compile with --features ratex):
or_engine_use_ratex(engine, 40.0f, 2.0f);
```

Without a renderer, formulas stay as TeX in the IR for the host to typeset.

---

## Build

### Desktop

```bash
cargo test --workspace
cargo run --release -p openrender-core --example bench
cargo run -p openrender-core --example stream
```

### C header

```bash
cargo install cbindgen
cbindgen --config cbindgen.toml --crate openrender-core --output include/engine.h
```

### Android

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
cargo install cargo-ndk
export ANDROID_NDK_HOME=/path/to/ndk

./scripts/build-android.sh --jni --latex
# or, on Windows:
./scripts/build-android.ps1 -WithJni -WithLatex
```

Outputs, per ABI (`arm64-v8a`, `armeabi-v7a`, `x86_64`):

```text
android/jniLibs/<abi>/libopenrender.so
android/jniLibs/static/<abi>/libopenrender.a
```

### Gradle

[`android/openrender.gradle.kts`](android/openrender.gradle.kts) registers a `cargo ndk` task and wires it into `mergeJniLibFolders`/`preBuild`, so the freshly built `.so` files land in the APK without a manual copy step.

### Live streaming demo (WASM)

`web/` is a self-contained live demo: the same Rust core compiled to `wasm32-unknown-unknown`, fed 1–4 characters per tick, with every delta applied to the DOM by `block_id`.

| Mid-stream (tail still growing) | Completed |
|---|---|
| ![Live streaming, mid-stream](web/stream-mid.png) | ![Live streaming, completed](web/stream-done.png) |

<video src="https://raw.githubusercontent.com/raine-x/OpenRender/main/web/demo.mp4" controls muted loop playsinline></video>

*Screen recording of the live demo: character-by-character input, incremental DOM updates and live kernel metrics. Direct file: [`web/demo.mp4`](web/demo.mp4).*

```bash
rustup target add wasm32-unknown-unknown
cargo build -p openrender-core --target wasm32-unknown-unknown --release --features ratex
cp target/wasm32-unknown-unknown/release/openrender.wasm web/engine.wasm
python -m http.server 8124 --directory web
# open http://127.0.0.1:8124/
```

The core wasm module is **301 KiB** without math. The demo build uses `--features ratex`, embedding KaTeX fonts, and is **4.4 MiB**; it exposes the same C ABI (`or_engine_*`) plus `or_alloc`/`or_free_buffer` for zero-glue string copying. The metrics panel reports kernel time per feed, op count, unstable-tail bytes, revision and a live view of the latest delta.

### Preview page

```bash
cargo run -p openrender-core --features ratex --example preview -- preview/delta.json
python -m http.server 8123 --directory preview
# open http://127.0.0.1:8123/
```

The page fetches the IR delta and renders it with a small, independent host renderer.

---

## Verification

| Check | Result |
|---|---|
| Unit + integration tests | 13 + 20, all passing |
| Doc tests | passing |
| Feature builds | `latex`, `jni`, `ratex` all compile |
| RaTeX rendering test | real PNG output with embedded fonts |
| C ABI | `examples/c/demo.c` compiled with MSVC and executed against `openrender.dll` |
| WASM demo | engine built for `wasm32-unknown-unknown`; live streaming exercised and captured |
| JNI exports | `Java_ai_openrender_NativeEngine_*` and `or_engine_*` verified in the binary |
| Formatting | `cargo fmt --check` clean |

```bash
cargo test --workspace
cargo test -p openrender-latex --features ratex
cargo build -p openrender-core --features jni
```

Android cross-compilation is scripted but requires an NDK; it is not executed in this repository's test environment.

---

## Security model

- **Navigation allow-list.** Only configured schemes (`http`, `https`, `file`, `mailto`, `tel`, plus host additions) survive. `javascript:`, `data:`, and anything unknown degrade to text. Images with rejected schemes degrade to their alt text.
- **No code execution.** HTML is data, never markup; there is no WebView, no JS engine, and no network access in the engine.
- **Resource ceilings.** Document bytes, block count and unstable-tail size are capped; the parser and IR builder are iterative, so nesting depth cannot overflow the native stack.
- **Panic-safe FFI.** Every exported function catches panics and reports `{"error": ...}` instead of unwinding across the boundary.
- **Deterministic ownership.** Strings are returned as heap `char *` and must be freed with `or_string_free`; callbacks copy media bytes before `free`.

---

## Known trade-offs

1. **Lists split at blank lines.** A blank line between items commits the list; ordered numbering is preserved from the source, but a UI may show two list blocks.
2. **Very long single paragraphs** are force-split at a word boundary once they exceed `max_tail_bytes`.
3. **Spill latency.** The final, partial line of a huge (> 8 KiB) code fence appears on the next newline.
4. **Complex containers** (tables, deeply nested quotes) may appear as simpler blocks mid-stream and are replaced via `put` when settled; ids remain stable.
5. **No video block yet.** Audio has a dedicated directive; video URLs currently fall back to image/link semantics.
6. **Footnote references need their definition in the same parse window.** A reference that settles before its definition arrives remains literal (`[^n]`) instead of becoming a footnote span. Keep the reference and definition adjacent in the stream (the demo does) or use one-shot rendering.
7. **JSON is verbose.** The IR is intentionally human-readable; a binary codec is on the roadmap.

---

## Roadmap

- Binary IR codec behind the same op model.
- `video` blocks/spans and poster support.
- Device benchmark matrix (ARM64, low-end Android, iOS).
- Fuzzing and property tests for streaming invariants.
- Inline checkpointing to reduce tail re-parsing for pathological single paragraphs.
- Swift / UniFFI bindings.

---

## Contributing

Issues and pull requests are welcome. The most useful contributions right now:

- **Real-device benchmark data** (SoC, Android version, corpus, `feed` size) posted as an issue.
- **Adversarial Markdown corpora** that break rendering or streaming invariants.
- **Host renderers** (Jetpack Compose, UIKit, terminals) that exercise the IR.

Please keep the core dependency-free beyond `pulldown-cmark`, and add a test for every behavior change. Run `cargo fmt` and `cargo test --workspace` before sending a PR.

---

## License

MIT. See [LICENSE](LICENSE).

---

## Why this matters

Rich text should not require a browser engine, a network connection, or a flagship phone. OpenRender is a small, deterministic, privacy-preserving core that lets any native surface — a chat app, a note editor, a terminal, an offline knowledge base — render Markdown incrementally, on device, with no telemetry and no JavaScript. That is the contribution: put capable text rendering within reach of every screen, including the modest ones.
