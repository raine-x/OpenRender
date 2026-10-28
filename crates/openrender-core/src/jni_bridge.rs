//! Android JNI bridge (feature `jni`).
//!
//! Kotlin side:
//!
//! ```kotlin
//! class NativeEngine {
//!     private external fun nativeNew(
//!         throttleMs: Int,
//!         maxTailBytes: Int,
//!         maxDocumentBytes: Long,
//!         enablePreviews: Boolean,
//!     ): Long
//!     private external fun nativeFree(handle: Long)
//!     private external fun nativeFeed(handle: Long, text: String): String?
//!     private external fun nativeFlush(handle: Long): String?
//!     private external fun nativeFinish(handle: Long): String?
//!     private external fun nativeReset(handle: Long)
//! }
//! ```
//!
//! The returned JSON is the same delta format as the C ABI.

use jni::objects::{JObject, JString};
use jni::sys::{jboolean, jint, jlong, jstring};
use jni::JNIEnv;

use crate::config::Config;
use crate::engine::Engine;
use crate::ir::Delta;

#[no_mangle]
pub extern "system" fn Java_ai_openrender_NativeEngine_nativeNew(
    _env: JNIEnv,
    _this: JObject,
    throttle_ms: jint,
    max_tail_bytes: jint,
    max_document_bytes: jlong,
    enable_previews: jboolean,
) -> jlong {
    let mut cfg = Config::default();
    cfg.throttle_ms = throttle_ms.max(0) as u32;
    if max_tail_bytes > 0 {
        cfg.max_tail_bytes = max_tail_bytes as usize;
    }
    if max_document_bytes > 0 {
        cfg.max_document_bytes = max_document_bytes as usize;
    }
    cfg.enable_previews = enable_previews != 0;
    Box::into_raw(Box::new(Engine::new(cfg))) as jlong
}

#[no_mangle]
pub extern "system" fn Java_ai_openrender_NativeEngine_nativeFree(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
) {
    if handle == 0 {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle as *mut Engine));
    }
}

#[no_mangle]
pub extern "system" fn Java_ai_openrender_NativeEngine_nativeFeed(
    mut env: JNIEnv,
    _this: JObject,
    handle: jlong,
    text: JString,
) -> jstring {
    let Some(engine) = (unsafe { engine_ref(handle) }) else {
        return std::ptr::null_mut();
    };
    let Ok(input) = env.get_string(&text) else {
        return std::ptr::null_mut();
    };
    let input: String = input.into();
    let json = engine.feed(&input).to_json();
    match env.new_string(json) {
        Ok(value) => value.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_ai_openrender_NativeEngine_nativeFlush(
    env: JNIEnv,
    _this: JObject,
    handle: jlong,
) -> jstring {
    let Some(engine) = (unsafe { engine_ref(handle) }) else {
        return std::ptr::null_mut();
    };
    match env.new_string(engine.flush().to_json()) {
        Ok(value) => value.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_ai_openrender_NativeEngine_nativeFinish(
    env: JNIEnv,
    _this: JObject,
    handle: jlong,
) -> jstring {
    let Some(engine) = (unsafe { engine_ref(handle) }) else {
        return std::ptr::null_mut();
    };
    match env.new_string(engine.finish().to_json()) {
        Ok(value) => value.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_ai_openrender_NativeEngine_nativeReset(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
) {
    if let Some(engine) = unsafe { engine_ref(handle) } {
        engine.reset();
    }
}

/// Static helper: render a complete document in one call.
#[no_mangle]
pub extern "system" fn Java_ai_openrender_NativeEngine_nativeRender(
    mut env: JNIEnv,
    _this: JObject,
    text: JString,
) -> jstring {
    let Ok(input) = env.get_string(&text) else {
        return std::ptr::null_mut();
    };
    let input: String = input.into();
    let delta = Engine::render_once(&input, &Config::default());
    match env.new_string(delta.to_json()) {
        Ok(value) => value.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_ai_openrender_NativeEngine_nativeEmptyDelta(
    env: JNIEnv,
    _this: JObject,
) -> jstring {
    match env.new_string(Delta::default().to_json()) {
        Ok(value) => value.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_ai_openrender_NativeEngine_nativeConfigure(
    _env: JNIEnv,
    _this: JObject,
    handle: jlong,
    parser_flags: jint,
    soft_break_as_space: jboolean,
    drop_html: jboolean,
    size_hints: jboolean,
    max_blocks: jlong,
) -> jboolean {
    let Some(engine) = (unsafe { engine_ref(handle) }) else {
        return 0;
    };
    let cfg = engine.config_mut();
    cfg.parser_flags = parser_flags.max(0) as u32;
    cfg.soft_break_as_space = soft_break_as_space != 0;
    cfg.drop_html = drop_html != 0;
    cfg.enable_size_hints = size_hints != 0;
    cfg.max_blocks = max_blocks.max(0) as u64;
    1
}

unsafe fn engine_ref<'a>(handle: jlong) -> Option<&'a mut Engine> {
    if handle == 0 {
        return None;
    }
    unsafe { (handle as *mut Engine).as_mut() }
}
