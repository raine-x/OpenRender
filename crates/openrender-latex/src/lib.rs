//! Optional math rendering for `openrender`.
//!
//! The core markdown engine always emits `latex_inline` / `latex_block`
//! instructions carrying the raw TeX source. When this crate is compiled in
//! (feature `latex` on `openrender-core`), the host can additionally attach a
//! renderer so every formula also ships a rasterized PNG.
//!
//! Two ways to plug a renderer:
//! 1. [`RatexRenderer`] — the built-in pure-Rust KaTeX-compatible engine
//!    (feature `ratex`). Produces PNG bytes with embedded KaTeX fonts, no
//!    JavaScript, no WebView.
//! 2. Any custom [`MathRenderer`] implementation, including one backed by C
//!    callbacks (see `openrender-core`'s C ABI).

use std::sync::Arc;

/// Output container format of a rendered formula.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathFormat {
    Png,
    Svg,
}

impl MathFormat {
    pub fn mime(self) -> &'static str {
        match self {
            MathFormat::Png => "image/png",
            MathFormat::Svg => "image/svg+xml",
        }
    }
}

/// A rendered formula, owned by the caller.
#[derive(Clone, Debug)]
pub struct RenderedMath {
    pub format: MathFormat,
    pub data: Vec<u8>,
    /// Pixel width of `data`.
    pub width: u32,
    /// Pixel height of `data`.
    pub height: u32,
    /// Distance in pixels from the top of the image to the formula baseline.
    pub baseline: u32,
}

/// Pluggable math renderer. Implementations must be thread-safe because a
/// single engine may be shared by a host's worker threads.
pub trait MathRenderer: Send + Sync + 'static {
    fn render(&self, tex: &str, display: bool) -> Option<RenderedMath>;
}

/// A renderer that never produces output; formulas stay source-only.
pub struct NoopRenderer;

impl MathRenderer for NoopRenderer {
    fn render(&self, _tex: &str, _display: bool) -> Option<RenderedMath> {
        None
    }
}

/// Convenience alias used by hosts that store renderers behind `Arc`.
pub type SharedMathRenderer = Arc<dyn MathRenderer>;

#[cfg(feature = "ratex")]
pub use ratex_impl::RatexRenderer;

#[cfg(feature = "ratex")]
mod ratex_impl {
    use super::{MathFormat, MathRenderer, RenderedMath};
    use ratex_layout::{layout, to_display_list, validate_layout, LayoutOptions};
    use ratex_parser::parse;
    use ratex_render::{render_to_png, RenderOptions};
    use ratex_types::color::Color;
    use ratex_types::math_style::MathStyle;

    /// Pure-Rust KaTeX-compatible renderer built on RaTeX.
    #[derive(Clone, Debug)]
    pub struct RatexRenderer {
        /// Pixels per em.
        pub font_size: f32,
        /// Padding around the formula, in pixels (pre-DPR).
        pub padding: f32,
        /// Raster scale factor; use the Android screen density for crisp output.
        pub device_pixel_ratio: f32,
    }

    impl Default for RatexRenderer {
        fn default() -> Self {
            Self {
                font_size: 40.0,
                padding: 6.0,
                device_pixel_ratio: 1.0,
            }
        }
    }

    impl RatexRenderer {
        /// Render a formula to PNG. Transparent background, KaTeX fonts embedded
        /// by the `ratex-render/embed-fonts` feature.
        pub fn render_png(&self, tex: &str, display: bool) -> Option<RenderedMath> {
            if tex.trim().is_empty() {
                return None;
            }
            let nodes = parse(tex).ok()?;
            let style = if display {
                MathStyle::Display
            } else {
                MathStyle::Text
            };
            let layout_options = LayoutOptions::default()
                .with_style(style)
                .with_color(Color::new(0.0, 0.0, 0.0, 1.0));
            let root = layout(&nodes, &layout_options);
            validate_layout(&root).ok()?;
            let list = to_display_list(&root);

            let options = RenderOptions {
                font_size: self.font_size.max(1.0),
                padding: self.padding.max(0.0),
                background_color: Color::new(0.0, 0.0, 0.0, 0.0),
                font_dir: String::new(),
                device_pixel_ratio: self.device_pixel_ratio.max(0.01),
            };
            let data = render_to_png(&list, &options).ok()?;

            let dpr = options.device_pixel_ratio;
            let em_px = options.font_size * dpr;
            let pad_px = options.padding * dpr;
            let width = ((list.width as f32 * em_px + 2.0 * pad_px).ceil() as u32).max(1);
            let height =
                (((list.height + list.depth) as f32 * em_px + 2.0 * pad_px).ceil() as u32).max(1);
            let baseline = (list.height as f32 * em_px + pad_px).ceil() as u32;

            Some(RenderedMath {
                format: MathFormat::Png,
                data,
                width,
                height,
                baseline,
            })
        }
    }

    impl MathRenderer for RatexRenderer {
        fn render(&self, tex: &str, display: bool) -> Option<RenderedMath> {
            self.render_png(tex, display)
        }
    }
}

#[cfg(all(test, feature = "ratex"))]
mod tests {
    use super::*;

    #[test]
    fn renders_png_with_embedded_fonts() {
        let renderer = RatexRenderer::default();
        let image = renderer
            .render(r"\frac{1}{2}", true)
            .expect("render formula");
        assert_eq!(image.format, MathFormat::Png);
        assert!(image.data.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(image.width > 0);
        assert!(image.height > 0);
        assert!(image.baseline <= image.height);
    }

    #[test]
    fn rejects_empty_formula() {
        let renderer = RatexRenderer::default();
        assert!(renderer.render("   ", false).is_none());
    }
}
