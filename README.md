# OpenRender

**Streaming-first Markdown engine in Rust** · **流式优先的 Rust Markdown 引擎**

[English](README.EN.md) · [简体中文](README.ZH.md)

![OpenRender IR preview](preview/preview.png)

<video src="https://raw.githubusercontent.com/raine-x/OpenRender/main/web/demo.mp4" controls muted loop playsinline></video>

*Live streaming demo · 实时流式演示（[MP4](web/demo.mp4)）*

OpenRender feeds on fragments, settles only blocks that can no longer change, and emits language-neutral JSON directives with stable block ids. No WebView, no JavaScript runtime, no GUI toolkit.

OpenRender 以碎片化文本为输入，只提交不会再变化的块，输出带稳定 block id 的语言无关 JSON 指令。无 WebView、无 JavaScript 运行时、无 GUI 工具包。

- Incremental parsing bounded by new text + an 8 KiB tail, not document size.
- C ABI, JNI and `wasm32` builds; optional RaTeX math to PNG.
- About 500 KiB native core; allow-listed URLs, no HTML execution.

---

**[Read the full README in English →](README.EN.md)**
**[阅读完整中文说明 →](README.ZH.md)**

License: MIT. See [LICENSE](LICENSE).
