use openrender::{Config, Engine};

fn new_engine() -> Engine {
    Engine::new(Config {
        throttle_ms: 0,
        ..Config::default()
    })
}

#[test]
fn paragraphs_update_in_place() {
    let mut engine = new_engine();
    let first = engine.feed("Hello").to_json();
    assert!(first.contains("\"type\":\"paragraph\""));
    assert!(!first.contains("Hello world"));

    let second = engine.feed(" world\n\n").to_json();
    assert!(second.contains("Hello world"));
    assert_eq!(engine.tail_bytes(), 0);
}

#[test]
fn unclosed_fence_renders_immediately() {
    let mut engine = new_engine();
    let json = engine.feed("```rust\nfn main() {").to_json();
    assert!(json.contains("\"type\":\"code_block\""));
    assert!(json.contains("\"lang\":\"rust\""));
    assert!(json.contains("\"closed\":false"));
}

#[test]
fn unclosed_emphasis_is_completed() {
    let mut engine = new_engine();
    let json = engine.feed("**bold").to_json();
    assert!(json.contains("\"bold\":true"));
    assert!(json.contains("\"text\":\"bold\""));
}

#[test]
fn finishes_with_tables_tasks_and_alerts() {
    let mut engine = new_engine();
    let mut json = String::new();
    json.push_str(&engine.feed("| a | b |\n|---|---|\n| 1 | 2 |\n\n").to_json());
    json.push_str(&engine.feed("- [x] done\n- [ ] todo\n\n").to_json());
    json.push_str(&engine.feed("> [!WARNING]\n> careful\n").to_json());
    json.push_str(&engine.finish().to_json());
    assert!(json.contains("\"type\":\"table\""));
    assert!(json.contains("\"checked\":true"));
    assert!(json.contains("\"checked\":false"));
    assert!(json.contains("\"alert\":\"warning\""));
}

#[test]
fn audio_becomes_its_own_block() {
    let mut engine = new_engine();
    let json = engine
        .feed("![voice](https://cdn.test/voice.mp3)\n\n")
        .to_json();
    assert!(json.contains("\"type\":\"audio\""));
    assert!(json.contains("\"url\":\"https://cdn.test/voice.mp3\""));
}

#[test]
fn preview_cards_only_for_configured_domains() {
    let cfg = Config {
        throttle_ms: 0,
        ..Config::default()
    };
    let mut engine = Engine::new(cfg.clone());
    let json = engine.feed("[Title](https://example.com/a)\n\n").to_json();
    assert!(!json.contains("link_preview"));

    let mut cfg = Config {
        throttle_ms: 0,
        ..Config::default()
    };
    cfg.add_preview_domain("example.com");
    let mut engine = Engine::new(cfg);
    let json = engine.feed("[Title](https://example.com/a)\n\n").to_json();
    assert!(json.contains("link_preview"));
    assert!(json.contains("\"title\":\"Title\""));
}

#[test]
fn unsafe_schemes_are_dropped() {
    let mut engine = new_engine();
    let json = engine
        .feed("[click](javascript:alert(1)) and ![x](data:image/png;base64,AAAA)\n\n")
        .to_json();
    assert!(!json.contains("javascript"));
    assert!(!json.contains("data:image"));
    assert!(json.contains("click"));
}

#[test]
fn custom_scheme_allowed_by_config() {
    let mut engine = new_engine();
    engine.config_mut().allow_scheme("agent");
    let json = engine.feed("[open](agent://resource/42)\n\n").to_json();
    assert!(json.contains("agent://resource/42"));
}

#[test]
fn json_is_escaped() {
    let mut engine = new_engine();
    let json = engine.feed("a\"b\\c\n\n").to_json();
    assert!(json.contains("a\\\"b\\\\c"));
}

#[test]
fn incremental_and_one_shot_agree_on_plain_documents() {
    let doc = "# Title\n\nFirst paragraph.\n\nSecond **paragraph**.\n\n```rust\nlet x = 1;\n```\n";

    let mut engine = new_engine();
    let mut concatenated = String::new();
    for chunk in doc.split_inclusive('\n') {
        concatenated.push_str(&engine.feed(chunk).to_json());
    }
    concatenated.push_str(&engine.finish().to_json());

    for needle in [
        "Title",
        "First paragraph.",
        "Second ",
        "paragraph",
        "let x = 1;",
    ] {
        assert!(concatenated.contains(needle), "missing {needle}");
    }

    let one_shot = Engine::render_once(doc, &Config::default()).to_json();
    for needle in ["Title", "First paragraph.", "let x = 1;"] {
        assert!(one_shot.contains(needle), "missing {needle} in one-shot");
    }
}

#[test]
fn latex_source_survives_without_renderer() {
    let mut engine = new_engine();
    let json = engine
        .feed("$E=mc^2$ and\n\n$$\n\\int_0^1 x\\,dx\n$$\n")
        .to_json();
    assert!(json.contains("latex_inline"));
    assert!(json.contains("E=mc^2"));
    assert!(json.contains("latex_block"));
    assert!(json.contains("\\\\int_0^1")); // JSON-escaped backslash
}

#[test]
fn throttle_coalesces_feeds() {
    let mut engine = Engine::new(Config {
        throttle_ms: 50,
        ..Config::default()
    });
    let first = engine.feed_at("Hello", 0).to_json();
    assert!(!first.contains("Hello"));
    let second = engine.feed_at(" world", 10).to_json();
    assert!(!second.contains("Hello"));
    let third = engine.feed_at("", 60).to_json();
    assert!(third.contains("Hello world"));
}

#[test]
fn document_size_cap_drops_extra_input() {
    let mut engine = Engine::new(Config {
        throttle_ms: 0,
        max_document_bytes: 8,
        ..Config::default()
    });
    engine.feed("12345678901234567890");
    assert_eq!(engine.total_bytes(), 8);
}

#[test]
fn c_abi_roundtrip() {
    use openrender::ffi::*;
    use std::ffi::CString;

    unsafe {
        let cfg = OrConfig {
            throttle_ms: 0,
            max_tail_bytes: 0,
            max_document_bytes: 0,
            enable_previews: false,
        };
        let engine = or_engine_new(&cfg);
        assert!(!engine.is_null());
        let text = CString::new("# Hi\n\nbo").unwrap();
        let json = or_engine_feed(engine, text.as_ptr(), text.as_bytes().len());
        assert!(!json.is_null());
        let first = std::ffi::CStr::from_ptr(json).to_str().unwrap().to_string();
        or_string_free(json);
        assert!(first.contains("Hi"));

        let text = CString::new("dy").unwrap();
        let json = or_engine_feed(engine, text.as_ptr(), text.as_bytes().len());
        let second = std::ffi::CStr::from_ptr(json).to_str().unwrap().to_string();
        or_string_free(json);
        assert!(second.contains("body"));

        let json = or_engine_finish(engine);
        assert!(!json.is_null());
        or_string_free(json);
        or_engine_free(engine);
    }
}

#[test]
fn soft_break_policy_is_configurable() {
    let mut engine = new_engine();
    engine.config_mut().soft_break_as_space = true;
    let json = engine.feed("a\nb\n\n").to_json();
    assert!(json.contains("\"text\":\"a b\""));
    assert!(!json.contains("softbreak"));
}

#[test]
fn html_can_be_dropped() {
    let mut engine = new_engine();
    engine.config_mut().drop_html = true;
    let json = engine
        .feed("<div>secret</div>\n\ntext <b>x</b>\n\n")
        .to_json();
    assert!(!json.contains("secret"));
    assert!(!json.contains("div"));
    assert!(json.contains("text"));

    let mut engine = new_engine();
    let json = engine.feed("<div>kept</div>\n\n").to_json();
    assert!(json.contains("kept"));
}

#[test]
fn size_hints_can_be_disabled() {
    let mut engine = new_engine();
    let json = engine
        .feed("![alt](https://x.test/a.png \"=10x20\")\n\n")
        .to_json();
    assert!(json.contains("\"width\":10"));

    let mut engine = new_engine();
    engine.config_mut().enable_size_hints = false;
    let json = engine
        .feed("![alt](https://x.test/a.png \"=10x20\")\n\n")
        .to_json();
    assert!(!json.contains("\"width\""));
    assert!(json.contains("\"title\":\"=10x20\""));
}

#[test]
fn audio_extensions_are_configurable() {
    let mut engine = new_engine();
    let json = engine.feed("![v](https://x.test/a.xyz)\n\n").to_json();
    assert!(json.contains("\"type\":\"image\""));

    let mut engine = new_engine();
    engine.config_mut().allow_audio_extension("xyz");
    let json = engine.feed("![v](https://x.test/a.xyz)\n\n").to_json();
    assert!(json.contains("\"type\":\"audio\""));
}

#[test]
fn parser_flags_turn_extensions_off() {
    let mut engine = new_engine();
    engine
        .config_mut()
        .set_parser_flag(openrender::config::parse_flags::TABLES, false);
    let json = engine.feed("| a | b |\n|---|---|\n| 1 | 2 |\n\n").to_json();
    assert!(!json.contains("\"type\":\"table\""));
    assert!(json.contains("paragraph"));
}

#[test]
fn block_cap_stops_accepting_input() {
    let mut engine = Engine::new(Config {
        throttle_ms: 0,
        max_blocks: 1,
        ..Config::default()
    });
    engine.feed("first\n\n");
    let accepted = engine.total_bytes();
    engine.feed("second paragraph that should be dropped\n\n");
    assert_eq!(engine.total_bytes(), accepted);
}
