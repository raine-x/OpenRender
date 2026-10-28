//! C ABI surface. Every function is panic-safe and never throws across the
//! FFI boundary.
//!
//! Ownership: any `char *` returned by this module is a NUL-terminated,
//! heap-allocated UTF-8 string and **must** be released with
//! [`or_string_free`]. Engine handles must be released with [`or_engine_free`].

use std::ffi::{c_char, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::config::{parse_flags, Config};
use crate::engine::Engine;

/// Parser feature bits for [`or_engine_set_parser_flags`].
///
/// Combine with bitwise OR in C, e.g.
/// `or_engine_set_parser_flags(engine, OrParseFlag_Tables | OrParseFlag_Math)`.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrParseFlag {
    Tables = 1,
    Strikethrough = 2,
    TaskLists = 4,
    Footnotes = 8,
    Gfm = 16,
    Math = 32,
    SmartPunctuation = 64,
}

/// Configuration passed to [`or_engine_new`]. Zero-valued fields fall back to
/// defaults, so `or_config_default()` or an all-zero struct are both valid.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OrConfig {
    pub throttle_ms: u32,
    pub max_tail_bytes: u32,
    pub max_document_bytes: u64,
    pub enable_previews: bool,
}

/// Opaque streaming engine handle.
pub struct OrEngine {
    inner: Engine,
}

/// A rendered formula returned by a host math-renderer callback.
///
/// `data` remains owned by the callback until the engine copies it and then
/// invokes [`OrMathFreeFn`]; the engine never keeps the pointer.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OrMathImage {
    pub data: *mut u8,
    pub len: usize,
    pub width: u32,
    pub height: u32,
    pub baseline: u32,
    /// 0 = PNG, 1 = SVG.
    pub format: u32,
}

pub type OrMathRenderFn = unsafe extern "C" fn(
    tex: *const c_char,
    display: bool,
    user_data: *mut c_void,
    out: *mut OrMathImage,
) -> bool;

pub type OrMathFreeFn = unsafe extern "C" fn(data: *mut u8, len: usize, user_data: *mut c_void);

#[no_mangle]
pub extern "C" fn or_config_default() -> OrConfig {
    let cfg = Config::default();
    OrConfig {
        throttle_ms: cfg.throttle_ms,
        max_tail_bytes: cfg.max_tail_bytes.min(u32::MAX as usize) as u32,
        max_document_bytes: cfg.max_document_bytes as u64,
        enable_previews: cfg.enable_previews,
    }
}

#[no_mangle]
pub extern "C" fn or_engine_version() -> *const c_char {
    b"0.1.0\0".as_ptr() as *const c_char
}

/// True when the library was compiled with the `latex` feature.
#[no_mangle]
pub extern "C" fn or_engine_has_latex() -> bool {
    cfg!(feature = "latex")
}

/// True when the built-in RaTeX renderer is available (`ratex` feature).
#[no_mangle]
pub extern "C" fn or_engine_has_ratex() -> bool {
    cfg!(feature = "ratex")
}

/// Create an engine. `cfg` may be NULL for defaults.
#[no_mangle]
pub extern "C" fn or_engine_new(cfg: *const OrConfig) -> *mut OrEngine {
    let config = config_from(cfg);
    Box::into_raw(Box::new(OrEngine {
        inner: Engine::new(config),
    }))
}

/// Destroy an engine. NULL is a no-op.
#[no_mangle]
pub unsafe extern "C" fn or_engine_free(engine: *mut OrEngine) {
    if engine.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(engine));
    }
}

#[no_mangle]
pub unsafe extern "C" fn or_engine_reset(engine: *mut OrEngine) {
    if let Some(engine) = engine.as_mut() {
        engine.inner.reset();
    }
}

/// Append UTF-8 text. Returns a JSON delta, or NULL if `engine` is NULL.
///
/// The delta is `{"v":1,"rev":N,"ops":[...]}`. Apply ops by block id:
///
/// - `{"op":"put","block":{...}}`      insert or replace a block
/// - `{"op":"append","id":N,"text":""}` append text to a streaming code block
/// - `{"op":"close","id":N}`           code block finished
/// - `{"op":"remove","id":N}`          block disappeared (shrinking tail)
///
/// When throttling merges this call into a pending window the delta carries an
/// empty `ops` array; call [`or_engine_flush`] (or the next feed) to drain it.
///
/// # Safety
/// `text` must point to `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn or_engine_feed(
    engine: *mut OrEngine,
    text: *const c_char,
    len: usize,
) -> *mut c_char {
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return std::ptr::null_mut();
    };
    let Some(text) = (unsafe { str_arg(text, len) }) else {
        return error_json("invalid_utf8");
    };
    let inner = &mut engine.inner;
    match catch_unwind(AssertUnwindSafe(|| inner.feed(text).to_json())) {
        Ok(json) => into_c_string(json),
        Err(_) => error_json("internal_error"),
    }
}

/// Deterministic variant of [`or_engine_feed`] using a caller-provided clock.
#[no_mangle]
pub unsafe extern "C" fn or_engine_feed_at(
    engine: *mut OrEngine,
    text: *const c_char,
    len: usize,
    now_ms: u64,
) -> *mut c_char {
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return std::ptr::null_mut();
    };
    let Some(text) = (unsafe { str_arg(text, len) }) else {
        return error_json("invalid_utf8");
    };
    let inner = &mut engine.inner;
    match catch_unwind(AssertUnwindSafe(|| inner.feed_at(text, now_ms).to_json())) {
        Ok(json) => into_c_string(json),
        Err(_) => error_json("internal_error"),
    }
}

/// Parse any throttled/ buffered text immediately.
#[no_mangle]
pub unsafe extern "C" fn or_engine_flush(engine: *mut OrEngine) -> *mut c_char {
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return std::ptr::null_mut();
    };
    let inner = &mut engine.inner;
    match catch_unwind(AssertUnwindSafe(|| inner.flush().to_json())) {
        Ok(json) => into_c_string(json),
        Err(_) => error_json("internal_error"),
    }
}

/// Commit the remaining tail and finish the document.
#[no_mangle]
pub unsafe extern "C" fn or_engine_finish(engine: *mut OrEngine) -> *mut c_char {
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return std::ptr::null_mut();
    };
    let inner = &mut engine.inner;
    match catch_unwind(AssertUnwindSafe(|| inner.finish().to_json())) {
        Ok(json) => into_c_string(json),
        Err(_) => error_json("internal_error"),
    }
}

#[no_mangle]
pub unsafe extern "C" fn or_engine_revision(engine: *const OrEngine) -> u64 {
    match unsafe { engine.as_ref() } {
        Some(engine) => engine.inner.revision(),
        None => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn or_engine_tail_bytes(engine: *const OrEngine) -> usize {
    match unsafe { engine.as_ref() } {
        Some(engine) => engine.inner.tail_bytes(),
        None => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn or_engine_total_bytes(engine: *const OrEngine) -> u64 {
    match unsafe { engine.as_ref() } {
        Some(engine) => engine.inner.total_bytes() as u64,
        None => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn or_engine_is_finished(engine: *const OrEngine) -> bool {
    match unsafe { engine.as_ref() } {
        Some(engine) => engine.inner.is_finished(),
        None => true,
    }
}

/// Allow an extra URL scheme (e.g. `agent`). Returns false on bad input.
#[no_mangle]
pub unsafe extern "C" fn or_engine_allow_scheme(
    engine: *mut OrEngine,
    scheme: *const c_char,
) -> bool {
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return false;
    };
    let Some(scheme) = (unsafe { cstr_arg(scheme) }) else {
        return false;
    };
    engine.inner.config_mut().allow_scheme(scheme);
    true
}

/// Register a host whose standalone links render as preview cards.
#[no_mangle]
pub unsafe extern "C" fn or_engine_add_preview_domain(
    engine: *mut OrEngine,
    domain: *const c_char,
) -> bool {
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return false;
    };
    let Some(domain) = (unsafe { cstr_arg(domain) }) else {
        return false;
    };
    engine.inner.config_mut().add_preview_domain(domain);
    true
}

/// Default parser feature mask (tables, strike, task lists, footnotes, GFM, math).
#[no_mangle]
pub extern "C" fn or_parse_flags_default() -> u32 {
    parse_flags::DEFAULT
}

/// Replace the parser feature mask. See [`OrParseFlag`].
#[no_mangle]
pub unsafe extern "C" fn or_engine_set_parser_flags(engine: *mut OrEngine, flags: u32) -> bool {
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return false;
    };
    engine.inner.config_mut().parser_flags = flags;
    true
}

/// Set the throttle window in milliseconds (`0` = render on arrival).
#[no_mangle]
pub unsafe extern "C" fn or_engine_set_throttle_ms(
    engine: *mut OrEngine,
    throttle_ms: u32,
) -> bool {
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return false;
    };
    engine.inner.config_mut().throttle_ms = throttle_ms;
    true
}

/// Set the unstable-tail byte cap.
#[no_mangle]
pub unsafe extern "C" fn or_engine_set_max_tail_bytes(
    engine: *mut OrEngine,
    max_tail_bytes: u32,
) -> bool {
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return false;
    };
    if max_tail_bytes > 0 {
        engine.inner.config_mut().max_tail_bytes = max_tail_bytes as usize;
    }
    true
}

/// Set the total-document byte cap.
#[no_mangle]
pub unsafe extern "C" fn or_engine_set_max_document_bytes(
    engine: *mut OrEngine,
    max_document_bytes: u64,
) -> bool {
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return false;
    };
    if max_document_bytes > 0 {
        engine.inner.config_mut().max_document_bytes = max_document_bytes as usize;
    }
    true
}

/// Set the committed-block cap (`0` disables it).
#[no_mangle]
pub unsafe extern "C" fn or_engine_set_max_blocks(engine: *mut OrEngine, max_blocks: u64) -> bool {
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return false;
    };
    engine.inner.config_mut().max_blocks = max_blocks;
    true
}

/// Enable/disable link preview cards.
#[no_mangle]
pub unsafe extern "C" fn or_engine_set_previews(engine: *mut OrEngine, enabled: bool) -> bool {
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return false;
    };
    engine.inner.config_mut().enable_previews = enabled;
    true
}

/// Render soft line breaks as spaces instead of `softbreak` spans.
#[no_mangle]
pub unsafe extern "C" fn or_engine_set_soft_break_as_space(
    engine: *mut OrEngine,
    enabled: bool,
) -> bool {
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return false;
    };
    engine.inner.config_mut().soft_break_as_space = enabled;
    true
}

/// Drop raw HTML instead of emitting it as literal text.
#[no_mangle]
pub unsafe extern "C" fn or_engine_set_drop_html(engine: *mut OrEngine, enabled: bool) -> bool {
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return false;
    };
    engine.inner.config_mut().drop_html = enabled;
    true
}

/// Enable/disable `=300x200` image size hints.
#[no_mangle]
pub unsafe extern "C" fn or_engine_set_size_hints(engine: *mut OrEngine, enabled: bool) -> bool {
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return false;
    };
    engine.inner.config_mut().enable_size_hints = enabled;
    true
}

/// Treat an extra file extension as audio (e.g. `"amr"`).
#[no_mangle]
pub unsafe extern "C" fn or_engine_allow_audio_extension(
    engine: *mut OrEngine,
    extension: *const c_char,
) -> bool {
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return false;
    };
    let Some(extension) = (unsafe { cstr_arg(extension) }) else {
        return false;
    };
    engine.inner.config_mut().allow_audio_extension(extension);
    true
}

/// Attach a host math renderer. `render` must return true and fill `out` on
/// success; the engine copies the bytes and then calls `free`. Returns false
/// when the library was compiled without the `latex` feature.
///
/// # Safety
/// Both function pointers must remain valid for the lifetime of the engine.
#[no_mangle]
pub unsafe extern "C" fn or_engine_set_math_renderer(
    engine: *mut OrEngine,
    render: OrMathRenderFn,
    free: OrMathFreeFn,
    user_data: *mut c_void,
) -> bool {
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return false;
    };

    #[cfg(feature = "latex")]
    {
        engine
            .inner
            .set_math_renderer(Some(std::sync::Arc::new(CallbackRenderer {
                render,
                free,
                user_data,
            })));
        true
    }

    #[cfg(not(feature = "latex"))]
    {
        let _ = (engine, render, free, user_data);
        false
    }
}

/// Detach the current math renderer. Returns false without the `latex`
/// feature.
#[no_mangle]
pub unsafe extern "C" fn or_engine_clear_math_renderer(engine: *mut OrEngine) -> bool {
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return false;
    };

    #[cfg(feature = "latex")]
    {
        engine.inner.set_math_renderer(None);
        true
    }

    #[cfg(not(feature = "latex"))]
    {
        let _ = engine;
        false
    }
}

/// Install the built-in RaTeX renderer. Returns false without the `ratex`
/// feature.
#[no_mangle]
pub unsafe extern "C" fn or_engine_use_ratex(
    engine: *mut OrEngine,
    font_size: f32,
    device_pixel_ratio: f32,
) -> bool {
    let Some(engine) = (unsafe { engine.as_mut() }) else {
        return false;
    };

    #[cfg(feature = "ratex")]
    {
        let mut renderer = openrender_latex::RatexRenderer::default();
        if font_size.is_finite() && font_size > 0.0 {
            renderer.font_size = font_size;
        }
        if device_pixel_ratio.is_finite() && device_pixel_ratio > 0.0 {
            renderer.device_pixel_ratio = device_pixel_ratio;
        }
        engine
            .inner
            .set_math_renderer(Some(std::sync::Arc::new(renderer)));
        true
    }

    #[cfg(not(feature = "ratex"))]
    {
        let _ = (engine, font_size, device_pixel_ratio);
        false
    }
}

/// One-shot render of a complete document. `cfg` may be NULL.
///
/// # Safety
/// `text` must point to `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn or_render(
    text: *const c_char,
    len: usize,
    cfg: *const OrConfig,
) -> *mut c_char {
    let Some(text) = (unsafe { str_arg(text, len) }) else {
        return error_json("invalid_utf8");
    };
    let config = config_from(cfg);
    match catch_unwind(AssertUnwindSafe(|| {
        Engine::render_once(text, &config).to_json()
    })) {
        Ok(json) => into_c_string(json),
        Err(_) => error_json("internal_error"),
    }
}

/// Allocate `len` writable bytes inside the engine's address space.
///
/// Intended for C bridges and WebAssembly hosts that need to copy UTF-8 input
/// before calling `or_engine_feed`. Release with [`or_free_buffer`].
#[no_mangle]
pub extern "C" fn or_alloc(len: usize) -> *mut u8 {
    if len == 0 {
        return std::ptr::null_mut();
    }
    match std::alloc::Layout::array::<u8>(len) {
        Ok(layout) => unsafe { std::alloc::alloc(layout) },
        Err(_) => std::ptr::null_mut(),
    }
}

/// Release a buffer returned by [`or_alloc`]. `ptr` may be NULL.
///
/// # Safety
/// `ptr` and `len` must come from a successful `or_alloc(len)` call.
#[no_mangle]
pub unsafe extern "C" fn or_free_buffer(ptr: *mut u8, len: usize) {
    if ptr.is_null() || len == 0 {
        return;
    }
    if let Ok(layout) = std::alloc::Layout::array::<u8>(len) {
        unsafe { std::alloc::dealloc(ptr, layout) };
    }
}

/// Release a string returned by this library. NULL is a no-op.
#[no_mangle]
pub unsafe extern "C" fn or_string_free(text: *mut c_char) {
    if text.is_null() {
        return;
    }
    unsafe {
        drop(CString::from_raw(text));
    }
}

#[cfg(feature = "latex")]
struct CallbackRenderer {
    render: OrMathRenderFn,
    free: OrMathFreeFn,
    user_data: *mut c_void,
}

#[cfg(feature = "latex")]
unsafe impl Send for CallbackRenderer {}
#[cfg(feature = "latex")]
unsafe impl Sync for CallbackRenderer {}

#[cfg(feature = "latex")]
impl openrender_latex::MathRenderer for CallbackRenderer {
    fn render(&self, tex: &str, display: bool) -> Option<openrender_latex::RenderedMath> {
        let tex = CString::new(tex).ok()?;
        let mut out = OrMathImage {
            data: std::ptr::null_mut(),
            len: 0,
            width: 0,
            height: 0,
            baseline: 0,
            format: 0,
        };
        let ok = unsafe { (self.render)(tex.as_ptr(), display, self.user_data, &mut out) };
        if !ok || out.data.is_null() || out.len == 0 {
            return None;
        }
        let data = unsafe { std::slice::from_raw_parts(out.data, out.len) }.to_vec();
        unsafe { (self.free)(out.data, out.len, self.user_data) };
        Some(openrender_latex::RenderedMath {
            format: if out.format == 1 {
                openrender_latex::MathFormat::Svg
            } else {
                openrender_latex::MathFormat::Png
            },
            data,
            width: out.width,
            height: out.height,
            baseline: out.baseline,
        })
    }
}

fn config_from(cfg: *const OrConfig) -> Config {
    let mut config = Config::default();
    let Some(cfg) = (unsafe { cfg.as_ref() }) else {
        return config;
    };
    config.throttle_ms = cfg.throttle_ms;
    if cfg.max_tail_bytes > 0 {
        config.max_tail_bytes = cfg.max_tail_bytes as usize;
    }
    if cfg.max_document_bytes > 0 {
        config.max_document_bytes = cfg.max_document_bytes as usize;
    }
    config.enable_previews = cfg.enable_previews;
    config
}

fn into_c_string(value: String) -> *mut c_char {
    match CString::new(value) {
        Ok(value) => value.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

fn error_json(message: &str) -> *mut c_char {
    into_c_string(format!(
        "{{\"v\":1,\"rev\":0,\"ops\":[],\"error\":\"{}\"}}",
        message
    ))
}

unsafe fn str_arg<'a>(ptr: *const c_char, len: usize) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len) };
    std::str::from_utf8(bytes).ok()
}

unsafe fn cstr_arg<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr) }.to_str().ok()
}
