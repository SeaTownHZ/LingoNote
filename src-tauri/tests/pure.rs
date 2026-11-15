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

// ---------- M2.5：不透明度与位置策略 ----------

#[test]
fn ui_opacity_defaults_and_clamps() {
    let config = StoredConfig::default();
    assert_eq!(config.ui.opacity, DEFAULT_OPACITY);
    assert_eq!(config.ui.opacity, 100);

    let mut config = StoredConfig::default();
    config.ui.opacity = 120;
    assert_eq!(normalize_config(config).ui.opacity, MAX_OPACITY);

    let mut config = StoredConfig::default();
    config.ui.opacity = 10;
    assert_eq!(normalize_config(config).ui.opacity, MIN_OPACITY);

    // 老配置没有 opacity 字段时使用默认 100，其余 ui 字段不受影响。
    let legacy: StoredConfig = serde_json::from_str(r#"{"ui": {"font_size": 18}}"#).unwrap();
    assert_eq!(legacy.ui.opacity, DEFAULT_OPACITY);
    assert_eq!(legacy.ui.font_size, 18);
}

#[test]
fn window_config_defaults_and_normalization() {
    let config = StoredConfig::default();
    assert_eq!(config.window.position_mode, POSITION_MODE_CURSOR);
    assert_eq!(config.window.anchor, DEFAULT_ANCHOR);
    assert_eq!(config.window.margin_x, DEFAULT_MARGIN);
    assert_eq!(config.window.margin_y, DEFAULT_MARGIN);
    assert_eq!(config.window.last_x, None);
    assert_eq!(config.window.last_y, None);

    let mut config = StoredConfig::default();
    config.window.position_mode = "floating".into();
    config.window.anchor = "middle".into();
    config.window.margin_x = -8;
    config.window.margin_y = 999;
    config.window.last_x = Some(-50);
    config.window.last_y = Some(5000);
    let normalized = normalize_config(config);
    assert_eq!(normalized.window.position_mode, POSITION_MODE_CURSOR);
    assert_eq!(normalized.window.anchor, DEFAULT_ANCHOR);
    assert_eq!(normalized.window.margin_x, MIN_MARGIN);
    assert_eq!(normalized.window.margin_y, MAX_MARGIN);
    // 记住的坐标不做范围裁剪，使用时再夹回工作区。
    assert_eq!(normalized.window.last_x, Some(-50));
    assert_eq!(normalized.window.last_y, Some(5000));

    let mut config = StoredConfig::default();
    config.window.position_mode = "REMEMBER".into();
    assert_eq!(
        normalize_config(config).window.position_mode,
        POSITION_MODE_REMEMBER
    );
}

#[test]
fn window_config_partial_json_uses_defaults_and_keeps_last_point() {
    // 老 v2 配置没有 window 字段：默认值补齐，仍能正常反序列化。
    let legacy: StoredConfig = serde_json::from_str(
        r#"{"schema_version": 2, "base_url": "https://api.deepseek.com/v1", "model": "deepseek-chat"}"#,
    )
    .unwrap();
    assert_eq!(legacy.window.position_mode, "cursor");
    assert_eq!(legacy.window.anchor, "bottom-right");
    assert_eq!(legacy.window.last_x, None);

    // 只带部分 window 字段：缺的用默认值，已有的 last_x/last_y 保留。
    let partial: StoredConfig = serde_json::from_str(
        r#"{"window": {"position_mode": "remember", "last_x": 12, "last_y": -30}}"#,
    )
    .unwrap();
    assert_eq!(partial.window.position_mode, "remember");
    assert_eq!(partial.window.anchor, DEFAULT_ANCHOR);
    assert_eq!(partial.window.margin_x, DEFAULT_MARGIN);
    assert_eq!(partial.window.last_x, Some(12));
    assert_eq!(partial.window.last_y, Some(-30));
}

#[test]
fn unknown_anchor_falls_back_to_bottom_right() {
    let work = WorkArea::new(0, 0, 1000, 800);
    let win = (200, 100);
    let margin = (10, 10);
    let expected = anchor_position("bottom-right", win, work, margin);
    assert_eq!(expected, (790, 690));
    assert_eq!(anchor_position("diagonal", win, work, margin), expected);
    assert_eq!(anchor_position("", win, work, margin), expected);

    let mut config = StoredConfig::default();
    config.window.anchor = "top-middle".into();
    assert_eq!(normalize_config(config).window.anchor, DEFAULT_ANCHOR);
}

#[test]
fn cursor_position_flips_when_bottom_right_has_no_room() {
    let work = WorkArea::new(0, 0, 1920, 1040);
    let win = (300, 200);

    // 空间充足：出现在光标右下 12px。
    assert_eq!(
        cursor_position((100, 100), win, work, CURSOR_GAP),
        (112, 112)
    );

    // 右下都放不下：横向、纵向同时翻转到光标左上。
    assert_eq!(
        cursor_position((1900, 1030), win, work, CURSOR_GAP),
        (1900 - 300 - 12, 1030 - 200 - 12)
    );

    // 只有右侧溢出：只翻转 x，y 保持右下偏移。
    assert_eq!(
        cursor_position((1910, 100), win, work, CURSOR_GAP),
        (1910 - 300 - 12, 112)
    );

    // 翻转后仍越界时由 clamp 夹回工作区。
    let small = WorkArea::new(0, 0, 500, 400);
    assert_eq!(cursor_position((310, 300), win, small, CURSOR_GAP), (0, 88));
}

#[test]
fn clamp_position_keeps_note_inside_work_area() {
    // 负坐标的左侧副屏。
    let work = WorkArea::new(-1920, 0, 0, 1040);
    let win = (400, 300);

    // 记住的点落在屏幕外：夹回工作区可放置范围。
    assert_eq!(clamp_position((-2500, -80), win, work), (-1920, 0));
    assert_eq!(clamp_position((120, 2000), win, work), (-400, 740));
    // 窗口会溢出右/下边缘时向左上夹回。
    assert_eq!(clamp_position((-100, 100), win, work), (-400, 100));

    // 窗口比工作区还大：贴左上，不产生反向区间。
    let small_work = WorkArea::new(0, 0, 300, 200);
    assert_eq!(
        clamp_position((150, 100), (500, 400), small_work),
        (0, 0)
    );
}

#[test]
fn anchor_position_covers_nine_grid() {
    let work = WorkArea::new(0, 0, 1200, 900);
    let win = (300, 200);
    let margin = (20, 30);

    let cases = [
        ("top-left", (20, 30)),
        ("top-center", (450, 30)),
        ("top-right", (880, 30)),
        ("center-left", (20, 350)),
        ("center", (450, 350)),
        ("center-right", (880, 350)),
        ("bottom-left", (20, 670)),
        ("bottom-center", (450, 670)),
        ("bottom-right", (880, 670)),
    ];
    for (anchor, expected) in cases {
        assert_eq!(
            anchor_position(anchor, win, work, margin),
            expected,
            "anchor {anchor} 位置不符合九宫格"
        );
    }
}

#[test]
fn anchor_position_applies_margin_and_clamps_oversized_window() {
    let work = WorkArea::new(0, 0, 1200, 900);
    let win = (300, 200);

    assert_eq!(
        anchor_position("bottom-right", win, work, (24, 40)),
        (876, 660)
    );
    assert_eq!(anchor_position("top-left", win, work, (0, 0)), (0, 0));
    // center 忽略边距，始终居中。
    assert_eq!(
        anchor_position("center", win, work, (400, 400)),
        (450, 350)
    );

    // 窗口比工作区还大时，锚点位置被夹回工作区（贴左上）。
    assert_eq!(
        anchor_position("bottom-right", (1500, 1000), work, (24, 40)),
        (0, 0)
    );
}

#[test]
fn config_json_contract_matches_spec_section_3() {
    let config = StoredConfig::default();
    let value = serde_json::to_value(&config).unwrap();

    assert_eq!(value["ui"]["opacity"], 100);
    assert_eq!(value["window"]["position_mode"], "cursor");
    assert_eq!(value["window"]["anchor"], "bottom-right");
    assert_eq!(value["window"]["margin_x"], 16);
    assert_eq!(value["window"]["margin_y"], 16);
    assert!(value["window"].get("last_x").is_some());
    assert_eq!(value["window"]["last_x"], Value::Null);
    assert_eq!(value["window"]["last_y"], Value::Null);
}
