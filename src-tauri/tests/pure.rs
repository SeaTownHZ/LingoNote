#[allow(dead_code)]
#[path = "../src/pure.rs"]
mod pure;

use pure::*;
use serde_json::Value;

#[test]
fn normalize_url_appends_endpoint() {
    assert_eq!(
        normalize_url("https://api.deepseek.com"),
        "https://api.deepseek.com/chat/completions"
    );
    assert_eq!(
        normalize_url("https://api.deepseek.com/v1/"),
        "https://api.deepseek.com/v1/chat/completions"
    );
    assert_eq!(
        normalize_url("https://example.com/chat/completions"),
        "https://example.com/chat/completions"
    );
}

#[test]
fn friendly_http_error_maps_common_statuses() {
    assert!(friendly_http_error(401, "").contains("API Key"));
    assert!(friendly_http_error(403, "").contains("无权访问"));
    assert!(friendly_http_error(404, "").contains("接口地址"));
    assert!(friendly_http_error(429, "").contains("请求太频繁"));
    assert!(friendly_http_error(500, "").contains("暂时不可用"));
}

#[test]
fn system_prompt_keeps_target_and_rules() {
    let prompt = system_prompt("English");
    assert!(prompt.contains("English"));
    assert!(prompt.contains("只输出译文"));
    assert!(prompt.contains("<source_text>"));
}

#[test]
fn custom_prompt_replaces_target_placeholder() {
    let prompt = resolve_prompt("把内容翻译成{target_language}，只输出译文。", "日本語");
    assert_eq!(prompt, "把内容翻译成日本語，只输出译文。");

    let fallback = resolve_prompt("   ", "English");
    assert!(fallback.contains("翻译成English"));
    assert!(fallback.contains("<source_text>"));
    assert!(!fallback.contains("{target_language}"));
}

#[test]
fn legacy_config_deserializes_with_defaults() {
    let json = r#"{
        "base_url": "https://api.deepseek.com",
        "api_key": "sk-legacy",
        "model": "deepseek-chat",
        "target_language": "简体中文"
    }"#;
    let config: StoredConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.legacy_api_key, "sk-legacy");
    assert_eq!(config.schema_version, 1);
    assert_eq!(config.hotkey, "Alt+Q");
    assert_eq!(config.ui.font_family, "SimSun");
    assert_eq!(config.ui.font_size, 16);
    assert_eq!(config.ui.radius, 12);
    assert!(config.behavior.hide_on_blur);
    assert!(!config.behavior.copy_restores_clipboard);
    assert!(!config.behavior.appear_focused);
    assert_eq!(config.behavior.max_chars, DEFAULT_MAX_CHARS);
    assert!(!config.history.enabled);
    assert!(!config.logging.enabled);
    assert_eq!(config.logging.level, "error");
}

#[test]
fn serialized_config_never_contains_legacy_key() {
    let mut config = StoredConfig::default();
    config.legacy_api_key = "sk-legacy".into();
    let text = serde_json::to_string(&config).unwrap();
    assert!(!text.contains("api_key"));
    assert!(!text.contains("sk-legacy"));
}

#[test]
fn config_defaults_are_user_facing_defaults() {
    let config = StoredConfig::default();
    assert_eq!(config.schema_version, SCHEMA_VERSION);
    assert_eq!(config.hotkey, "Alt+Q");
    assert_eq!(config.target_language, "简体中文");
    assert_eq!(config.ui.background, "#FFFFFF");
    assert_eq!(config.ui.foreground, "#111111");
    assert_eq!(config.ui.font_family, "SimSun");
    assert_eq!(config.ui.radius, 12);
    assert!(config.behavior.hide_on_blur);
    assert!(!config.behavior.copy_restores_clipboard);
    assert_eq!(config.behavior.max_chars, DEFAULT_MAX_CHARS);
    assert_eq!(config.history.limit, MAX_HISTORY as u32);
    assert!(!config.history.enabled);
}

#[test]
fn normalize_upgrades_schema_and_clamps_values() {
    let mut config = StoredConfig::default();
    config.schema_version = 1;
    config.ui.font_size = 2;
    config.ui.line_height = 9.0;
    config.ui.radius = 100;
    config.ui.background = "white".into();
    config.behavior.max_chars = 0;
    config.history.limit = 999;
    config.logging.level = "LOUD".into();
    config.hotkey = "  ".into();

    let normalized = normalize_config(config);
    assert_eq!(normalized.schema_version, SCHEMA_VERSION);
    assert_eq!(normalized.ui.font_size, 10);
    assert_eq!(normalized.ui.line_height, 3.0);
    assert_eq!(normalized.ui.radius, 32);
    assert_eq!(normalized.ui.background, "#FFFFFF");
    assert_eq!(normalized.behavior.max_chars, DEFAULT_MAX_CHARS);
    assert_eq!(normalized.history.limit, MAX_HISTORY as u32);
    assert_eq!(normalized.logging.level, "error");
    assert_eq!(normalized.hotkey, "Alt+Q");
}

#[test]
fn export_excludes_api_key_and_marks_timestamp() {
    let config = StoredConfig::default();
    let value = build_export(&config, "2026-11-14T18:30:00+08:00");
    let text = serde_json::to_string(&value).unwrap();
    assert!(!text.contains("api_key"));
    assert_eq!(value["schema_version"], Value::from(SCHEMA_VERSION));
    assert_eq!(value["exported_at"], "2026-11-14T18:30:00+08:00");
    assert_eq!(value["ui"]["line_height"], 1.65);
    assert!(value.get("prompt").is_some());
    assert!(value.get("history").is_some());
}

#[test]
fn import_validates_schema_and_url() {
    let valid = serde_json::json!({
        "schema_version": 2,
        "base_url": "https://api.deepseek.com/v1",
        "model": "deepseek-chat"
    })
    .to_string();
    let config = parse_import(&valid).unwrap();
    assert_eq!(config.schema_version, SCHEMA_VERSION);
    assert_eq!(config.hotkey, "Alt+Q");

    let future = serde_json::json!({
        "schema_version": 99,
        "base_url": "https://api.deepseek.com/v1",
        "model": "deepseek-chat"
    })
    .to_string();
    assert!(parse_import(&future).unwrap_err().contains("版本"));

    let bad_url = serde_json::json!({
        "schema_version": 2,
        "base_url": "ftp://example.com",
        "model": "deepseek-chat"
    })
    .to_string();
    assert!(parse_import(&bad_url).unwrap_err().contains("Base URL"));

    let missing_model = serde_json::json!({
        "schema_version": 2,
        "base_url": "https://example.com/v1"
    })
    .to_string();
    assert!(parse_import(&missing_model).unwrap_err().contains("模型"));
}

#[test]
fn history_is_newest_first_and_trimmed() {
    fn entry(id: u64) -> HistoryEntry {
        HistoryEntry {
            id,
            time: "2026-11-14T18:00:00+08:00".into(),
            source: "hello".into(),
            translation: "你好".into(),
            target_language: "简体中文".into(),
            model: "deepseek-chat".into(),
            duration_ms: 42,
        }
    }

    let mut entries = Vec::new();
    for id in 1..=5 {
        push_history(&mut entries, entry(id), 3);
    }
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].id, 5);
    assert_eq!(entries[2].id, 3);
}

#[test]
fn atomic_write_replaces_existing_file() {
    let dir = std::env::temp_dir().join(format!("lingonote-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.json");

    write_atomic(&path, "first").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
    write_atomic(&path, "second").unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");

    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp"))
        .collect();
    assert!(leftovers.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn log_level_and_line_are_sanitized() {
    assert_eq!(parse_log_level("debug"), 3);
    assert_eq!(parse_log_level("INFO"), 2);
    assert_eq!(parse_log_level("error"), 1);
    assert_eq!(parse_log_level("other"), 1);

    let line = log_line("2026-11-14 18:30:00.000", "ERROR", "first\nsecond\r\nthird");
    assert_eq!(line, "[2026-11-14 18:30:00.000][ERROR] first second third\n");
}

#[test]
fn max_chars_is_clamped_to_supported_range() {
    let mut config = StoredConfig::default();
    config.behavior.max_chars = 10;
    assert_eq!(normalize_config(config).behavior.max_chars, MIN_MAX_CHARS);

    let mut config = StoredConfig::default();
    config.behavior.max_chars = 999_999;
    assert_eq!(
        normalize_config(config).behavior.max_chars,
        MAX_MAX_CHARS
    );
}
