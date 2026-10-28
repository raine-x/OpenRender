//! Emit the IR for the browser preview page.
//!
//! ```bash
//! cargo run -p openrender-core --features ratex --example preview -- preview/delta.json
//! ```
//!
//! The example feeds one document through the streaming engine, applies the
//! deltas exactly like a host would, and writes the final block list as a
//! single JSON delta for `preview/index.html`.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use openrender::{Block, Config, Delta, Engine, Op};

const DOC: &str = r##"# OpenRender 渲染预览

这是由 Rust 引擎输出的 **IR** 直接渲染的页面：*斜体*、~~删除线~~、`行内代码`、[链接](https://example.com/docs)、行内公式 $E=mc^2$。

> [!TIP]
> 这是 GFM 警告块。它来自 `> [!TIP]` 语法，宿主可以完全自定义视觉样式。

## 流式状态

- 已提交块只解析一次
- 未稳定尾部每帧重渲染
  - 超大围栏走 append 协议
  - 尾部受 8 KiB 上限约束
- [x] 标记任务完成
- [ ] 处理剩余任务

## 能力矩阵

| 能力 | 状态 | 说明 |
|:---|:---:|---:|
| 流式解析 | 完成 | 增量分块 |
| 表格 | 完成 | 对齐支持 |
| LaTeX | 可选 | RaTeX · PNG |

## 代码

```rust
let mut engine = Engine::new(Config::default());
let delta = engine.feed("# Hello\n\nstreaming");
println!("{}", delta.to_json());
```

---

块级公式：

$$
\int_0^1 x^2\,dx = \frac{1}{3}
$$

![终端渲染示意图](https://trae-api-cn.mchost.guru/api/ide/v1/text_to_image?prompt=minimal%20light%20developer%20workspace%20illustration%2C%20abstract%20markdown%20document%20streaming%20into%20rounded%20blocks%2C%20soft%20blue%20accents%2C%20clean%20flat%20design&image_size=landscape_4_3 "=640x360")

![语音输入](https://cdn.example.com/audio/voice-note.mp3)

[OpenRender：流式 Markdown 引擎](https://example.com/openrender "在单一内核里完成解析、容错与增量输出")

脚注也能流式出现[^1]。
[^1]: 这是一个脚注定义块。
"##;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "preview/delta.json".to_string());

    let mut cfg = Config::default();
    cfg.throttle_ms = 0;
    cfg.add_preview_domain("example.com");
    cfg.add_preview_domain("openrender.dev");

    let mut engine = Engine::new(cfg);
    #[cfg(feature = "ratex")]
    engine.use_ratex();

    let mut blocks: BTreeMap<u64, Block> = BTreeMap::new();
    absorb(engine.feed(DOC), &mut blocks);
    absorb(engine.finish(), &mut blocks);

    let delta = Delta {
        rev: engine.revision(),
        ops: blocks.into_values().map(Op::Put).collect(),
    };
    let json = delta.to_json();

    if let Some(parent) = Path::new(&path).parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(&path, &json).expect("write delta json");
    eprintln!(
        "wrote {} blocks (rev {}) to {} ({} bytes)",
        delta.ops.len(),
        delta.rev,
        path,
        json.len()
    );
}

fn absorb(delta: Delta, blocks: &mut BTreeMap<u64, Block>) {
    for op in delta.ops {
        match op {
            Op::Put(block) => {
                blocks.insert(block.id, block);
            }
            Op::Append { id, text } => {
                if let Some(block) = blocks.get_mut(&id) {
                    block.text.push_str(&text);
                }
            }
            Op::Close { .. } => {}
            Op::Remove { id } => {
                blocks.remove(&id);
            }
        }
    }
}
