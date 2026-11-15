use std::str::FromStr;

use global_hotkey::hotkey::HotKey;

#[test]
fn hotkey_parser_accepts_expected_formats() {
    assert!(HotKey::from_str("Alt+Q").is_ok());
    assert!(HotKey::from_str("Ctrl+Shift+T").is_ok());
    assert!(HotKey::from_str("Alt+Tab").is_ok());
}

#[test]
fn hotkey_parser_rejects_invalid_text() {
    assert!(HotKey::from_str("not a key").is_err());
    assert!(HotKey::from_str("").is_err());
}
