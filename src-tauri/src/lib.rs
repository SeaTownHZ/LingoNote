use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::menu::{CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, LogicalSize, PhysicalSize, State, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder, Wry,
};
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

mod logging;
mod pure;
use pure::{
    anchor_position, build_export, clamp_note_size, clamp_position, cursor_position,
    format_recent_label, friendly_http_error, is_newer, merge_window_for_save, normalize_config,
    normalize_url, normalize_version, parse_import, preview_text, push_history, resolve_prompt,
    write_atomic, ConfigInput, ConfigView, HistoryEntry, StoredConfig, WorkArea, ANCHOR_CUSTOM,
    CURSOR_GAP, DEFAULT_ANCHOR, DEFAULT_MAX_CHARS, POSITION_MODE_ANCHOR, POSITION_MODE_REMEMBER,
};

const KEYRING_SERVICE: &str = "LingoNote";
const KEYRING_USER: &str = "api-key";

/// GitHub Releases 仓库与更新检查端点（SPEC 3.2）。
const UPDATE_REPO: &str = "SeaTownHZ/LingoNote";
const DEFAULT_UPDATE_API: &str = "https://api.github.com";
const UPDATE_TIMEOUT: Duration = Duration::from_secs(8);
const UPDATE_NOTES_LIMIT: usize = 4000;

static CONFIG_WARNING: Mutex<Option<String>> = Mutex::new(None);

/// 托盘 id（最近翻译点击复制后 3 秒恢复默认 tooltip，SPEC 4.5）。
const TRAY_ID: &str = "lingonote-tray";

/// 取点遮罩窗口 label / URL（SPEC 4.4）。
const POSITION_PICK_LABEL: &str = "position-pick";
const POSITION_PICK_URL: &str = "position-pick.html";

/// 最近翻译容量与空剪贴板错误文案（SPEC 4.1 / 4.5）。
const RECENT_CAPACITY: usize = 5;
const EMPTY_CLIPBOARD_ERROR: &str = "剪贴板里没有可翻译的文本。复制一段文字后再按热键。";

/// 取点成功后的便签预览时长。
const PREVIEW_DURATION: Duration = Duration::from_millis(800);
/// 「已复制」tooltip 的展示时长。
const TRAY_TOOLTIP_DURATION: Duration = Duration::from_millis(3000);

/// 取点遮罩是否打开：期间屏蔽全局热键与托盘「翻译剪贴板」（SPEC 4.4）。
static PICKING: AtomicBool = AtomicBool::new(false);
/// 便签预览代际：每次 show_note 自增，取点预览超时后只在代数未变时隐藏。
static PREVIEW_GENERATION: AtomicU64 = AtomicU64::new(0);
/// tooltip 代际：连续点击最近翻译时只让最后一次的 3 秒计时生效。
static TRAY_TOOLTIP_GENERATION: AtomicU64 = AtomicU64::new(0);

/// 会话内最近翻译（仅内存，不落盘，SPEC 4.5）。
static RECENT: Mutex<VecDeque<RecentEntry>> = Mutex::new(VecDeque::new());

#[derive(Debug, Clone, Serialize)]
pub struct RecentEntry {
    /// `HH:MM`。
    pub at: String,
    pub text: String,
}

/// 选区直读结果（SPEC 4.1）。
#[derive(Debug, Clone, Serialize)]
pub struct SourceText {
    pub text: String,
    pub from_selection: bool,
    pub clipboard_restored: bool,
}

#[derive(Debug, Clone, Serialize)]
struct HotkeyStatus {
    hotkey: String,
    error: Option<String>,
    paused: bool,
}

#[derive(Debug, Clone)]
struct RuntimeConfig {
    base_url: String,
    model: String,
    target_language: String,
    api_key: String,
    prompt: String,
}

struct HotkeyInner {
    current: Option<Shortcut>,
    text: String,
    error: Option<String>,
    paused: bool,
}

impl Default for HotkeyInner {
    fn default() -> Self {
        Self {
            current: None,
            text: "Alt+Q".into(),
            error: None,
            paused: false,
        }
    }
}

#[derive(Default)]
struct HotkeyState(Mutex<HotkeyInner>);

#[derive(Default)]
struct TranslationState(Arc<AtomicU64>);

struct TrayMenu {
    pause: CheckMenuItem<Wry>,
    autostart: CheckMenuItem<Wry>,
}

#[derive(Default)]
struct TrayState {
    /// 当前菜单里的勾选/禁用项，用于免重建地更新勾选状态。
    menu: Mutex<Option<TrayMenu>>,
    /// 保留 TrayIcon 句柄，便于翻译成功后动态 `set_menu`（SPEC 4.5）。
    tray: Mutex<Option<TrayIcon<Wry>>>,
}

#[derive(Debug, Clone, Serialize)]
struct AppInfo {
    version: String,
    identifier: String,
    config_dir: String,
    config_path: String,
    history_path: String,
    log_dir: String,
    hotkey_paused: bool,
}

/// 更新检查结果（SPEC 3.2 契约）。
#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub has_update: bool,
    pub url: String,
    pub notes: String,
    pub published_at: String,
}

// ---------- 配置与路径 ----------

fn config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("无法定位配置目录：{e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建配置目录：{e}"))?;
    Ok(dir)
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(config_dir(app)?.join("config.json"))
}

fn history_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(config_dir(app)?.join("history.json"))
}

fn app_log_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_log_dir()
        .map_err(|e| format!("无法定位日志目录：{e}"))
}

fn load_api_key() -> Result<Option<String>, String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|e| format!("无法访问系统凭据库：{e}"))?;
    match entry.get_password() {
        Ok(password) if !password.trim().is_empty() => Ok(Some(password)),
        Ok(_) => Ok(None),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(format!("读取 API Key 失败：{error}")),
    }
}

fn save_api_key(api_key: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|e| format!("无法访问系统凭据库：{e}"))?;
    entry
        .set_password(api_key)
        .map_err(|e| format!("保存 API Key 失败：{e}"))
}

fn write_stored_config(app: &AppHandle, config: &StoredConfig) -> Result<(), String> {
    let path = config_path(app)?;
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    write_atomic(&path, &text).map_err(|e| format!("保存配置失败：{e}"))
}

fn load_stored_config(app: &AppHandle) -> StoredConfig {
    let Ok(path) = config_path(app) else {
        return StoredConfig::default();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return StoredConfig::default();
    };

    let parsed: StoredConfig = match serde_json::from_str(&text) {
        Ok(config) => config,
        Err(error) => {
            let backup = path.with_file_name("config.bad.json");
            let _ = std::fs::copy(&path, &backup);
            logging::error(format!(
                "配置文件解析失败，已备份到 {}：{error}",
                backup.display()
            ));
            if let Ok(mut warning) = CONFIG_WARNING.lock() {
                *warning = Some(
                    "配置文件已损坏，已备份为 config.bad.json，并使用默认配置。".to_string(),
                );
            }
            let defaults = StoredConfig::default();
            let _ = write_stored_config(app, &defaults);
            return defaults;
        }
    };

    let original: Option<Value> = serde_json::from_str(&text).ok();
    let mut config = normalize_config(parsed);
    let mut should_write = original.as_ref() != serde_json::to_value(&config).ok().as_ref();

    if !config.legacy_api_key.trim().is_empty() {
        match save_api_key(&config.legacy_api_key) {
            Ok(()) => {
                config.legacy_api_key.clear();
                should_write = true;
            }
            Err(error) => {
                logging::error(format!("M0 明文 API Key 迁移失败：{error}"));
                // 迁移失败时不要写回，避免把唯一一份明文 Key 清掉。
                should_write = false;
            }
        }
    }

    if should_write {
        let _ = write_stored_config(app, &config);
    }

    config
}

fn configure_logging(app: &AppHandle, config: &StoredConfig) {
    match app_log_dir(app) {
        Ok(dir) => logging::configure(dir, config.logging.enabled, &config.logging.level),
        Err(error) => {
            let _ = error;
        }
    }
}

fn build_config_view(app: &AppHandle, state: &HotkeyState) -> ConfigView {
    let stored = load_stored_config(app);
    let runtime = state
        .0
        .lock()
        .map(|inner| Some((inner.text.clone(), inner.paused)))
        .unwrap_or(None);
    let (hotkey, paused) = runtime.unwrap_or((stored.hotkey.clone(), stored.behavior.pause_hotkey));
    let behavior = pure::BehaviorConfig {
        pause_hotkey: paused,
        ..stored.behavior
    };
    let has_api_key = load_api_key().ok().flatten().is_some();
    let warning = CONFIG_WARNING
        .lock()
        .ok()
        .and_then(|warning| warning.clone());

    ConfigView {
        schema_version: stored.schema_version,
        base_url: stored.base_url,
        model: stored.model,
        target_language: stored.target_language,
        hotkey,
        ui: stored.ui,
        window: stored.window,
        behavior,
        prompt: stored.prompt.system,
        default_prompt: pure::DEFAULT_PROMPT.to_string(),
        history: stored.history,
        logging: stored.logging,
        has_api_key,
        warning,
    }
}

// ---------- 配置命令 ----------

#[tauri::command]
fn get_config(app: AppHandle, state: State<'_, HotkeyState>) -> ConfigView {
    build_config_view(&app, &state)
}

#[tauri::command]
fn save_config(
    app: AppHandle,
    state: State<'_, HotkeyState>,
    config: ConfigInput,
) -> Result<ConfigView, String> {
    if let Some(key) = config.api_key.as_deref() {
        if !key.trim().is_empty() {
            save_api_key(key.trim())?;
        }
    }

    let paused = state
        .0
        .lock()
        .map(|inner| inner.paused)
        .unwrap_or(false);

    let mut stored = load_stored_config(&app);
    // 设置页自动保存不带 last_x/last_y（None），必须保留磁盘上记住的位置；
    // fixed_x/fixed_y 的 None 则表示“清除取点”，由 SPEC 3 单独约定，不保留。
    let window = merge_window_for_save(config.window, &stored.window);
    stored.base_url = config.base_url;
    stored.model = config.model;
    stored.target_language = config.target_language;
    stored.ui = config.ui;
    stored.window = window;
    stored.behavior = config.behavior;
    // 暂停状态以运行时为准，避免设置页的旧值把托盘里的暂停覆盖掉。
    stored.behavior.pause_hotkey = paused;
    stored.prompt = config.prompt;
    stored.history = config.history;
    stored.logging = config.logging;
    stored.legacy_api_key.clear();

    let stored = normalize_config(stored);
    write_stored_config(&app, &stored)?;
    cache_note_position_mode(&stored.window.position_mode);
    configure_logging(&app, &stored);

    let _ = app.emit_to("note", "config-changed", ());
    logging::info("settings saved");
    Ok(build_config_view(&app, &state))
}

#[tauri::command]
fn dismiss_config_warning() {
    if let Ok(mut warning) = CONFIG_WARNING.lock() {
        *warning = None;
    }
}

#[tauri::command]
fn get_app_info(app: AppHandle, state: State<'_, HotkeyState>) -> AppInfo {
    let dir = config_dir(&app).unwrap_or_default();
    let log_dir = app_log_dir(&app).unwrap_or_default();
    let paused = state
        .0
        .lock()
        .map(|inner| inner.paused)
        .unwrap_or(false);

    AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        identifier: app.config().identifier.clone(),
        config_dir: dir.to_string_lossy().to_string(),
        config_path: dir.join("config.json").to_string_lossy().to_string(),
        history_path: dir.join("history.json").to_string_lossy().to_string(),
        log_dir: log_dir.to_string_lossy().to_string(),
        hotkey_paused: paused,
    }
}

#[tauri::command]
fn export_config(app: AppHandle, path: String) -> Result<String, String> {
    let stored = load_stored_config(&app);
    let exported_at = chrono::Local::now().to_rfc3339();
    let value = build_export(&stored, &exported_at);
    let text = serde_json::to_string_pretty(&value).map_err(|e| format!("生成配置失败：{e}"))?;
    write_atomic(Path::new(&path), &text).map_err(|e| format!("写入导出文件失败：{e}"))?;
    logging::info(format!("config exported: {path}"));
    Ok(path)
}

#[tauri::command]
fn import_config(
    app: AppHandle,
    state: State<'_, HotkeyState>,
    path: String,
) -> Result<ConfigView, String> {
    let text = std::fs::read_to_string(&path).map_err(|e| format!("读取导入文件失败：{e}"))?;
    let imported = parse_import(&text)?;

    if let Ok(current) = config_path(&app) {
        if current.exists() {
            let backup = current.with_file_name("config.backup.json");
            std::fs::copy(&current, &backup)
                .map_err(|e| format!("备份当前配置失败：{e}"))?;
        }
    }

    write_stored_config(&app, &imported)?;
    cache_note_position_mode(&imported.window.position_mode);
    if let Err(error) = sync_hotkey_from_config(
        &app,
        &state,
        &imported.hotkey,
        imported.behavior.pause_hotkey,
    ) {
        logging::error(format!("导入后应用热键失败：{error}"));
        if let Ok(mut inner) = state.0.lock() {
            inner.error = Some(error);
        }
    }
    update_pause_check(&app, imported.behavior.pause_hotkey);
    configure_logging(&app, &imported);
    let _ = app.emit_to("note", "config-changed", ());
    logging::info(format!("config imported: {path}"));
    Ok(build_config_view(&app, &state))
}

#[tauri::command]
fn reset_config(app: AppHandle, state: State<'_, HotkeyState>) -> Result<ConfigView, String> {
    if let Ok(current) = config_path(&app) {
        if current.exists() {
            let backup = current.with_file_name("config.backup.json");
            std::fs::copy(&current, &backup)
                .map_err(|e| format!("备份当前配置失败：{e}"))?;
        }
    }

    let defaults = StoredConfig::default();
    write_stored_config(&app, &defaults)?;
    cache_note_position_mode(&defaults.window.position_mode);

    if let Err(error) = sync_hotkey_from_config(&app, &state, &defaults.hotkey, false) {
        logging::error(format!("恢复默认热键失败：{error}"));
        if let Ok(mut inner) = state.0.lock() {
            inner.error = Some(error);
        }
    }
    update_pause_check(&app, false);
    configure_logging(&app, &defaults);
    let _ = app.emit_to("note", "config-changed", ());
    logging::info("config reset to defaults");
    Ok(build_config_view(&app, &state))
}

// ---------- 更新检查（SPEC 3.2） ----------

/// 更新 API 基址：默认 GitHub；`LINGONOTE_UPDATE_API` 仅供本地烟测/CI 覆盖。
fn update_api_base() -> String {
    match std::env::var("LINGONOTE_UPDATE_API") {
        Ok(value) => {
            let trimmed = value.trim().trim_end_matches('/');
            if trimmed.is_empty() {
                DEFAULT_UPDATE_API.to_string()
            } else {
                trimmed.to_string()
            }
        }
        Err(_) => DEFAULT_UPDATE_API.to_string(),
    }
}

fn update_check_error(status: u16) -> String {
    match status {
        403 | 429 => "GitHub 接口访问受限，请稍后重试".into(),
        404 => "尚未找到已发布的版本".into(),
        500..=599 => "更新服务暂时不可用".into(),
        code => format!("HTTP {code}"),
    }
}

/// 仅放行 GitHub HTTPS 链接，并拒绝会被 `cmd /C start` 解析成命令的字符。
fn is_allowed_release_url(url: &str) -> bool {
    url.starts_with("https://github.com/")
        && !url.chars().any(|c| {
            c.is_whitespace()
                || c.is_control()
                || matches!(
                    c,
                    '"' | '\'' | '`' | '&' | '|' | '<' | '>' | '^' | '%' | '\\'
                )
        })
}

fn open_in_browser(url: &str) -> std::io::Result<std::process::ExitStatus> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;

        // 避免 `cmd` 的控制台窗口在打开浏览器时闪一下。
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).status()
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(url).status()
    }
}

/// 检查 GitHub Releases 最新版本；任何失败都返回中文 `Err`，不 panic、不重试。
#[tauri::command]
async fn check_update() -> Result<UpdateInfo, String> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let request_url = format!(
        "{}/repos/{UPDATE_REPO}/releases/latest",
        update_api_base()
    );

    let client = reqwest::Client::builder()
        .timeout(UPDATE_TIMEOUT)
        .build()
        .map_err(|error| format!("创建更新检查客户端失败：{error}"))?;

    let response = client
        .get(&request_url)
        .header(reqwest::header::USER_AGENT, format!("LingoNote/{current}"))
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| format!("连接更新服务失败：{error}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(update_check_error(status.as_u16()));
    }

    let value: Value = response
        .json()
        .await
        .map_err(|error| format!("解析更新信息失败：{error}"))?;

    let tag_name = value
        .get("tag_name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if tag_name.is_empty() {
        return Err("更新信息缺少版本号".into());
    }
    let latest = normalize_version(tag_name);
    if latest.is_empty() {
        return Err("更新信息缺少版本号".into());
    }

    let raw_url = value
        .get("html_url")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    // 契约要求 url 以 https://github.com/ 开头；上游字段异常时回落到仓库 Releases 页。
    let url = if is_allowed_release_url(raw_url) {
        raw_url.to_string()
    } else {
        format!("https://github.com/{UPDATE_REPO}/releases/latest")
    };

    let notes: String = value
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .take(UPDATE_NOTES_LIMIT)
        .collect();
    let published_at = value
        .get("published_at")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let has_update = is_newer(&latest, &current);

    logging::info(format!(
        "update check: current={current} latest={latest} has_update={has_update}"
    ));

    Ok(UpdateInfo {
        current,
        latest,
        has_update,
        url,
        notes,
        published_at,
    })
}

/// 在系统浏览器打开 Release 页；只接受 GitHub 链接，成功后写日志供验收取证。
#[tauri::command]
fn open_release_page(url: String) -> Result<(), String> {
    let url = url.trim();
    if !url.starts_with("https://github.com/") {
        return Err("拒绝打开非 GitHub 链接".into());
    }
    if !is_allowed_release_url(url) {
        return Err("链接包含非法字符".into());
    }

    let status = open_in_browser(url).map_err(|error| format!("打开浏览器失败：{error}"))?;
    if !status.success() {
        return Err(format!("打开浏览器失败（退出码 {status}）"));
    }

    logging::info(format!("open release page: {url}"));
    Ok(())
}

// ---------- 热键 ----------

#[tauri::command]
fn get_hotkey_status(state: State<'_, HotkeyState>) -> HotkeyStatus {
    let inner = state.0.lock().unwrap();
    HotkeyStatus {
        hotkey: inner.text.clone(),
        error: inner.error.clone(),
        paused: inner.paused,
    }
}

fn persist_hotkey_pause(app: &AppHandle, paused: bool, text: &str) -> Result<(), String> {
    let mut stored = load_stored_config(app);
    stored.hotkey = text.to_string();
    stored.behavior.pause_hotkey = paused;
    write_stored_config(app, &stored)
}

#[tauri::command]
fn set_hotkey(
    app: AppHandle,
    state: State<'_, HotkeyState>,
    hotkey: String,
) -> Result<HotkeyStatus, String> {
    let text = hotkey.trim().to_string();
    if text.is_empty() {
        return Err("热键不能为空".into());
    }
    let new_shortcut =
        Shortcut::from_str(&text).map_err(|e| format!("热键格式不正确：{e}"))?;

    let mut inner = state.0.lock().map_err(|_| "热键状态锁失败".to_string())?;

    if inner.paused {
        inner.text = text.clone();
        inner.error = None;
        let paused = inner.paused;
        drop(inner);
        persist_hotkey_pause(&app, paused, &text)?;
        let _ = app.emit_to("note", "config-changed", ());
        let _ = app.emit_to(
            "settings",
            "hotkey-changed",
            HotkeyStatus {
                hotkey: text.clone(),
                error: None,
                paused,
            },
        );
        return Ok(HotkeyStatus {
            hotkey: text,
            error: None,
            paused,
        });
    }

    if inner.current == Some(new_shortcut) {
        inner.text = text.clone();
        inner.error = None;
        drop(inner);
        persist_hotkey_pause(&app, false, &text)?;
        return Ok(HotkeyStatus {
            hotkey: text,
            error: None,
            paused: false,
        });
    }

    app.global_shortcut()
        .register(new_shortcut)
        .map_err(|e| format!("热键不可用（可能被其他程序占用）：{e}"))?;

    if let Some(old) = inner.current {
        let _ = app.global_shortcut().unregister(old);
    }
    inner.current = Some(new_shortcut);
    inner.text = text.clone();
    inner.error = None;
    drop(inner);

    persist_hotkey_pause(&app, false, &text)?;
    let _ = app.emit_to("note", "config-changed", ());
    let _ = app.emit_to(
        "settings",
        "hotkey-changed",
        HotkeyStatus {
            hotkey: text.clone(),
            error: None,
            paused: false,
        },
    );
    logging::info(format!("hotkey changed: {text}"));

    Ok(HotkeyStatus {
        hotkey: text,
        error: None,
        paused: false,
    })
}

fn apply_hotkey_paused(app: &AppHandle, paused: bool) -> Result<HotkeyStatus, String> {
    let state = app.state::<HotkeyState>();
    let mut inner = state.0.lock().map_err(|_| "热键状态锁失败".to_string())?;

    if paused {
        if let Some(current) = inner.current.take() {
            let _ = app.global_shortcut().unregister(current);
        }
        inner.paused = true;
        inner.error = None;
    } else if inner.paused {
        let shortcut = Shortcut::from_str(&inner.text)
            .map_err(|e| format!("热键格式不正确：{e}"))?;
        app.global_shortcut()
            .register(shortcut)
            .map_err(|e| format!("恢复热键失败（可能被其他程序占用）：{e}"))?;
        inner.current = Some(shortcut);
        inner.paused = false;
        inner.error = None;
    }

    let status = HotkeyStatus {
        hotkey: inner.text.clone(),
        error: inner.error.clone(),
        paused: inner.paused,
    };
    drop(inner);

    persist_hotkey_pause(app, status.paused, &status.hotkey)?;
    update_pause_check(app, status.paused);
    let _ = app.emit_to("note", "config-changed", ());
    let _ = app.emit_to("settings", "hotkey-changed", status.clone());
    logging::info(format!("hotkey paused={}", status.paused));
    Ok(status)
}

#[tauri::command]
fn set_hotkey_paused(app: AppHandle, paused: bool) -> Result<HotkeyStatus, String> {
    apply_hotkey_paused(&app, paused)
}

fn sync_hotkey_from_config(
    app: &AppHandle,
    state: &HotkeyState,
    text: &str,
    paused: bool,
) -> Result<(), String> {
    let mut inner = state.0.lock().map_err(|_| "热键状态锁失败".to_string())?;

    if paused {
        if let Some(current) = inner.current.take() {
            let _ = app.global_shortcut().unregister(current);
        }
        inner.text = text.to_string();
        inner.paused = true;
        inner.error = None;
        return Ok(());
    }

    let shortcut = Shortcut::from_str(text).map_err(|e| format!("热键格式不正确：{e}"))?;
    if !inner.paused && inner.current == Some(shortcut) {
        inner.text = text.to_string();
        inner.error = None;
        return Ok(());
    }

    app.global_shortcut()
        .register(shortcut)
        .map_err(|e| format!("热键不可用（可能被其他程序占用）：{e}"))?;
    if let Some(old) = inner.current.replace(shortcut) {
        if old != shortcut {
            let _ = app.global_shortcut().unregister(old);
        }
    }
    inner.text = text.to_string();
    inner.paused = false;
    inner.error = None;
    Ok(())
}

fn register_initial_hotkey(app: &AppHandle, stored: &StoredConfig) {
    let state = app.state::<HotkeyState>();
    let mut inner = state.0.lock().unwrap();
    inner.text = stored.hotkey.clone();
    inner.paused = stored.behavior.pause_hotkey;

    if inner.paused {
        inner.current = None;
        inner.error = None;
        logging::info(format!("hotkey paused by config: {}", stored.hotkey));
        return;
    }

    match Shortcut::from_str(&stored.hotkey) {
        Ok(shortcut) => match app.global_shortcut().register(shortcut) {
            Ok(()) => {
                inner.current = Some(shortcut);
                inner.error = None;
                logging::info(format!("hotkey registered: {}", stored.hotkey));
            }
            Err(error) => {
                inner.error = Some(format!("热键 {} 注册失败：{error}", stored.hotkey));
                logging::error(format!("hotkey register failed: {error}"));
            }
        },
        Err(error) => {
            inner.error = Some(format!("热键格式不正确：{error}"));
            logging::error(format!("hotkey parse failed: {error}"));
        }
    }
}

// ---------- 窗口 ----------

#[tauri::command]
fn open_settings(app: AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("settings")
        .ok_or_else(|| "设置窗口不存在（应由 Rust setup 或配置创建）".to_string())?;
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn close_settings(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("settings") {
        window.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn hide_note(window: WebviewWindow) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};

        let handle = window
            .window_handle()
            .map_err(|e| format!("无法获取窗口句柄：{e}"))?
            .as_raw();
        let hwnd = match handle {
            RawWindowHandle::Win32(h) => h.hwnd.get() as *mut _,
            _ => return Err("当前平台不是 Win32".into()),
        };
        unsafe {
            ShowWindow(hwnd, SW_HIDE);
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        window.hide().map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[tauri::command]
fn show_note(window: WebviewWindow) -> Result<(), String> {
    // 任何一次正式显示都让等待中的取点预览失效，避免 800ms 后误隐藏译文。
    PREVIEW_GENERATION.fetch_add(1, Ordering::SeqCst);
    let app = window.app_handle().clone();
    // 一次读取配置：定位只依赖这一份快照。
    let stored = load_stored_config(&app);
    // 顺带刷新拖动跟踪用的模式缓存，配置文件被外部改动后也能跟上。
    cache_note_position_mode(&stored.window.position_mode);
    let appear_focused = stored.behavior.appear_focused;

    #[cfg(target_os = "windows")]
    let window_config = stored.window;

    #[cfg(target_os = "windows")]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
        use windows_sys::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        };
        use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetCursorPos, GetForegroundWindow, GetSystemMetrics, GetWindowThreadProcessId,
            SetForegroundWindow, SetWindowPos, ShowWindow, HWND_TOPMOST, SM_CXSCREEN,
            SM_CYSCREEN, SWP_NOACTIVATE, SW_SHOWNOACTIVATE,
        };

        let handle = window
            .window_handle()
            .map_err(|e| format!("无法获取窗口句柄：{e}"))?
            .as_raw();
        let hwnd: HWND = match handle {
            RawWindowHandle::Win32(h) => h.hwnd.get() as *mut _,
            _ => return Err("当前平台不是 Win32".into()),
        };

        let size = window.outer_size().map_err(|e| e.to_string())?;
        let win_w = size.width as i32;
        let win_h = size.height as i32;
        let previous_foreground = unsafe { GetForegroundWindow() };

        unsafe {
            let mut pt = POINT::default();
            let (cursor_x, cursor_y) = if GetCursorPos(&mut pt) == 0 {
                (0, 0)
            } else {
                (pt.x, pt.y)
            };

            // 显示器选择：cursor/anchor 用光标所在屏；remember 用保存点最近的屏；
            // custom 用取点最近的屏（越界时夹到该屏工作区，AC-37）。
            let monitor_point = match window_config.position_mode.as_str() {
                POSITION_MODE_REMEMBER => match (window_config.last_x, window_config.last_y) {
                    (Some(last_x), Some(last_y)) => POINT {
                        x: last_x,
                        y: last_y,
                    },
                    _ => POINT {
                        x: cursor_x,
                        y: cursor_y,
                    },
                },
                POSITION_MODE_ANCHOR if window_config.anchor == ANCHOR_CUSTOM => {
                    match (window_config.fixed_x, window_config.fixed_y) {
                        (Some(fixed_x), Some(fixed_y)) => POINT {
                            x: fixed_x,
                            y: fixed_y,
                        },
                        _ => POINT {
                            x: cursor_x,
                            y: cursor_y,
                        },
                    }
                }
                _ => POINT {
                    x: cursor_x,
                    y: cursor_y,
                },
            };

            let mut monitor: MONITORINFO = std::mem::zeroed();
            monitor.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            let hmon = MonitorFromPoint(monitor_point, MONITOR_DEFAULTTONEAREST);
            let fallback = RECT {
                left: 0,
                top: 0,
                right: GetSystemMetrics(SM_CXSCREEN),
                bottom: GetSystemMetrics(SM_CYSCREEN),
            };
            let work = if !hmon.is_null() && GetMonitorInfoW(hmon, &mut monitor) != 0 {
                monitor.rcWork
            } else {
                fallback
            };

            let work_area = WorkArea::new(work.left, work.top, work.right, work.bottom);
            let win = (win_w, win_h);
            let cursor = (cursor_x, cursor_y);
            let (x, y) = match window_config.position_mode.as_str() {
                POSITION_MODE_ANCHOR if window_config.anchor == ANCHOR_CUSTOM => {
                    match (window_config.fixed_x, window_config.fixed_y) {
                        // 取点坐标即左上角；越界时统一夹回工作区（AC-37）。
                        (Some(fixed_x), Some(fixed_y)) => {
                            clamp_position((fixed_x, fixed_y), win, work_area)
                        }
                        // 归一化已保证 custom 必有坐标；这里仍防御性回退右下角。
                        _ => anchor_position(
                            DEFAULT_ANCHOR,
                            win,
                            work_area,
                            (window_config.margin_x, window_config.margin_y),
                        ),
                    }
                }
                POSITION_MODE_ANCHOR => anchor_position(
                    &window_config.anchor,
                    win,
                    work_area,
                    (window_config.margin_x, window_config.margin_y),
                ),
                POSITION_MODE_REMEMBER => match (window_config.last_x, window_config.last_y) {
                    (Some(last_x), Some(last_y)) => {
                        clamp_position((last_x, last_y), win, work_area)
                    }
                    _ => cursor_position(cursor, win, work_area, CURSOR_GAP),
                },
                _ => cursor_position(cursor, win, work_area, CURSOR_GAP),
            };

            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                x,
                y,
                win_w,
                win_h,
                SWP_NOACTIVATE,
            );

            if !appear_focused && !previous_foreground.is_null() && previous_foreground != hwnd {
                let _ = SetForegroundWindow(previous_foreground);

                let previous_addr = previous_foreground as isize;
                let note_addr = hwnd as isize;
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(150));
                    let previous = previous_addr as HWND;
                    let note = note_addr as HWND;
                    if previous.is_null() || GetForegroundWindow() != note {
                        return;
                    }
                    let foreground_thread =
                        GetWindowThreadProcessId(previous, std::ptr::null_mut());
                    let current_thread = GetCurrentThreadId();
                    let attached = if foreground_thread != 0
                        && foreground_thread != current_thread
                    {
                        AttachThreadInput(current_thread, foreground_thread, 1) != 0
                    } else {
                        false
                    };
                    let _ = SetForegroundWindow(previous);
                    if attached {
                        AttachThreadInput(current_thread, foreground_thread, 0);
                    }
                });
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        window.show().map_err(|e| e.to_string())?;
    }

    if appear_focused {
        let _ = window.set_focus();
    }

    Ok(())
}

// ---------- 便签尺寸、钉住与取点（SPEC 4.2–4.4） ----------

#[tauri::command]
fn set_note_size(window: WebviewWindow, width: u32, height: u32) -> Result<(), String> {
    let (width, height) = clamp_note_size(width, height);
    window
        .set_size(LogicalSize::new(width, height))
        .map_err(|error| format!("调整便签尺寸失败：{error}"))?;

    let app = window.app_handle();
    let mut stored = load_stored_config(app);
    if stored.window.note_width == width && stored.window.note_height == height {
        return Ok(());
    }
    stored.window.note_width = width;
    stored.window.note_height = height;
    let stored = normalize_config(stored);
    // 尺寸只落盘，不广播 config-changed（SPEC 4.2）。
    write_stored_config(app, &stored)
}

#[tauri::command]
fn set_note_pinned(app: AppHandle, pinned: bool) -> Result<(), String> {
    let mut stored = load_stored_config(&app);
    stored.window.pinned = pinned;
    let stored = normalize_config(stored);
    write_stored_config(&app, &stored)?;
    let _ = app.emit_to("note", "config-changed", ());
    let _ = app.emit_to("settings", "config-changed", ());
    logging::info(format!("note pinned set to {pinned}"));
    Ok(())
}

/// 取点遮罩覆盖的显示器：Windows 用光标所在屏的 `rcMonitor`（物理像素，允许负坐标）。
fn pick_monitor_rect(app: &AppHandle) -> (i32, i32, u32, u32) {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetCursorPos, GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN,
        };

        let _ = app;
        unsafe {
            let mut point = POINT { x: 0, y: 0 };
            let _ = GetCursorPos(&mut point);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
            if !monitor.is_null() && GetMonitorInfoW(monitor, &mut info) != 0 {
                let rect = info.rcMonitor;
                return (
                    rect.left,
                    rect.top,
                    (rect.right - rect.left).max(1) as u32,
                    (rect.bottom - rect.top).max(1) as u32,
                );
            }
            (
                0,
                0,
                GetSystemMetrics(SM_CXSCREEN).max(1) as u32,
                GetSystemMetrics(SM_CYSCREEN).max(1) as u32,
            )
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(Some(monitor)) = app.primary_monitor() {
            let position = monitor.position();
            let size = monitor.size();
            return (
                position.x,
                position.y,
                size.width.max(1),
                size.height.max(1),
            );
        }
        (0, 0, 1280, 720)
    }
}

/// 取点时的全局光标坐标（物理像素）。
fn pick_cursor_position(app: &AppHandle) -> (i32, i32) {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

        let _ = app;
        unsafe {
            let mut point = POINT { x: 0, y: 0 };
            if GetCursorPos(&mut point) != 0 {
                (point.x, point.y)
            } else {
                (0, 0)
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        app.cursor_position()
            .map(|position| (position.x.round() as i32, position.y.round() as i32))
            .unwrap_or((0, 0))
    }
}

#[tauri::command]
fn begin_position_pick(app: AppHandle) -> Result<(), String> {
    if let Some(existing) = app.get_webview_window(POSITION_PICK_LABEL) {
        PICKING.store(true, Ordering::SeqCst);
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(());
    }

    PICKING.store(true, Ordering::SeqCst);
    // Tauri 命令在 WebView2 的 IPC 回调里同步执行；直接 build() 会在等待窗口创建时
    // 与主线程互等（集成测试实测整机冻结）。这里先返回，再把创建任务排到主线程执行。
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(60));
        let inner = handle.clone();
        if let Err(error) = handle.run_on_main_thread(move || {
            if let Err(error) = create_position_pick_window(&inner) {
                PICKING.store(false, Ordering::SeqCst);
                logging::error(format!("打开取点遮罩失败：{error}"));
                let _ = inner.emit_to("settings", "position-pick-cancelled", ());
            }
        }) {
            // 调度本身失败时也必须解除热键屏蔽，否则用户只能重启应用。
            PICKING.store(false, Ordering::SeqCst);
            logging::error(format!("调度取点遮罩失败：{error}"));
            let _ = app.emit_to("settings", "position-pick-cancelled", ());
        }
    });
    Ok(())
}

/// 真正创建遮罩窗口；只在主线程调用（`begin_position_pick` 通过
/// `run_on_main_thread` 排队），避免与 IPC 回调互相等待。
fn create_position_pick_window(app: &AppHandle) -> Result<(), String> {
    if let Some(existing) = app.get_webview_window(POSITION_PICK_LABEL) {
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(());
    }

    let (left, top, width, height) = pick_monitor_rect(app);

    let window = WebviewWindowBuilder::new(
        app,
        POSITION_PICK_LABEL,
        WebviewUrl::App(POSITION_PICK_URL.into()),
    )
    .title("选择便签位置")
    .inner_size(320.0, 240.0)
    .position(0.0, 0.0)
    .transparent(true)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .shadow(false)
    .visible(false)
    .focused(true)
    .build()
    .map_err(|error| format!("打开取点遮罩失败：{error}"))?;

    // builder 的尺寸/位置参数是逻辑像素；这里按物理像素精确覆盖显示器。
    let _ = window.set_position(PhysicalPosition::new(left, top));
    let _ = window.set_size(PhysicalSize::new(width, height));

    let handle = app.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Destroyed = event {
            // 用户用 Alt+F4 等方式直接关掉遮罩时按取消处理；confirm/cancel
            // 会先把 PICKING 置 false，因此不会重复发事件。
            if PICKING.swap(false, Ordering::SeqCst) {
                let _ = handle.emit_to("settings", "position-pick-cancelled", ());
            }
        }
    });

    window
        .show()
        .map_err(|error| format!("显示取点遮罩失败：{error}"))?;
    let _ = window.set_focus();
    Ok(())
}

#[tauri::command]
fn cancel_position_pick(app: AppHandle) -> Result<(), String> {
    PICKING.store(false, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window(POSITION_PICK_LABEL) {
        let _ = window.close();
    }
    let _ = app.emit_to("settings", "position-pick-cancelled", ());
    Ok(())
}

#[tauri::command]
fn confirm_position_pick(app: AppHandle) -> Result<(), String> {
    let (x, y) = pick_cursor_position(&app);

    let mut stored = load_stored_config(&app);
    stored.window.position_mode = POSITION_MODE_ANCHOR.to_string();
    stored.window.anchor = ANCHOR_CUSTOM.to_string();
    stored.window.fixed_x = Some(x);
    stored.window.fixed_y = Some(y);
    let stored = normalize_config(stored);
    if let Err(error) = write_stored_config(&app, &stored) {
        // 落盘失败也要退出取点态，避免热键一直处于屏蔽状态。
        PICKING.store(false, Ordering::SeqCst);
        if let Some(window) = app.get_webview_window(POSITION_PICK_LABEL) {
            let _ = window.close();
        }
        let _ = app.emit_to("settings", "position-pick-cancelled", ());
        return Err(error);
    }

    PICKING.store(false, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window(POSITION_PICK_LABEL) {
        let _ = window.close();
    }
    let _ = app.emit_to("settings", "position-picked", json!({ "x": x, "y": y }));

    // 便签隐藏时显示 800ms 作为取点预览；期间若有正式翻译显示则让位。
    if let Some(note) = app.get_webview_window("note") {
        if !note.is_visible().unwrap_or(false) {
            if let Err(error) = show_note(note.clone()) {
                logging::error(format!("取点预览显示便签失败：{error}"));
                return Ok(());
            }
            let generation = PREVIEW_GENERATION.load(Ordering::SeqCst);
            let handle = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(PREVIEW_DURATION);
                if PREVIEW_GENERATION.load(Ordering::SeqCst) != generation {
                    return;
                }
                let restore = handle.clone();
                let _ = handle.run_on_main_thread(move || {
                    if PREVIEW_GENERATION.load(Ordering::SeqCst) != generation {
                        return;
                    }
                    if let Some(note) = restore.get_webview_window("note") {
                        let _ = hide_note(note);
                    }
                });
            });
        }
    }

    Ok(())
}

// ---------- 便签位置记忆 ----------

/// note 窗口当前位置模式的缓存：拖动时 Moved 每帧触发，不必每帧读磁盘配置。
static NOTE_POSITION_MODE: Mutex<String> = Mutex::new(String::new());
/// 去抖窗口内最新的便签坐标（物理像素）。
static NOTE_MOVE_PENDING: Mutex<Option<(i32, i32)>> = Mutex::new(None);
/// 是否已有去抖线程在等待落盘。
static NOTE_MOVE_DEBOUNCING: AtomicBool = AtomicBool::new(false);

/// 拖动停止约 600ms 后再落盘，避免拖动过程中每帧写配置。
const NOTE_MOVE_DEBOUNCE: Duration = Duration::from_millis(600);

fn cache_note_position_mode(mode: &str) {
    if let Ok(mut cached) = NOTE_POSITION_MODE.lock() {
        *cached = mode.to_string();
    }
}

fn note_remembers_position() -> bool {
    NOTE_POSITION_MODE
        .lock()
        .map(|cached| cached.as_str() == POSITION_MODE_REMEMBER)
        .unwrap_or(false)
}

/// 记录一次 Moved：更新待写坐标，并确保只有一个去抖线程在跑。
fn schedule_note_position_save(app: AppHandle, x: i32, y: i32) {
    if let Ok(mut pending) = NOTE_MOVE_PENDING.lock() {
        *pending = Some((x, y));
    }
    if NOTE_MOVE_DEBOUNCING.swap(true, Ordering::SeqCst) {
        return;
    }

    std::thread::spawn(move || loop {
        std::thread::sleep(NOTE_MOVE_DEBOUNCE);

        let pending = NOTE_MOVE_PENDING
            .lock()
            .ok()
            .and_then(|mut value| value.take());
        if let Some((x, y)) = pending {
            persist_note_position(&app, x, y);
        }

        let has_newer = NOTE_MOVE_PENDING
            .lock()
            .map(|value| value.is_some())
            .unwrap_or(false);
        if has_newer {
            continue;
        }

        NOTE_MOVE_DEBOUNCING.store(false, Ordering::SeqCst);
        // 置 false 的瞬间可能刚好有新事件写入，重新接管，避免丢失最后一次位置。
        let raced = NOTE_MOVE_PENDING
            .lock()
            .map(|value| value.is_some())
            .unwrap_or(false);
        if raced && !NOTE_MOVE_DEBOUNCING.swap(true, Ordering::SeqCst) {
            continue;
        }
        break;
    });
}

/// 只改磁盘配置里的 window.last_x/last_y；其它字段以磁盘当前值为准，且不广播事件。
fn persist_note_position(app: &AppHandle, x: i32, y: i32) {
    let mut stored = load_stored_config(app);
    // 拖动期间可能已切回其它模式，落盘前再确认一次。
    if stored.window.position_mode != POSITION_MODE_REMEMBER {
        return;
    }
    if stored.window.last_x == Some(x) && stored.window.last_y == Some(y) {
        return;
    }

    stored.window.last_x = Some(x);
    stored.window.last_y = Some(y);
    let stored = normalize_config(stored);
    if let Err(error) = write_stored_config(app, &stored) {
        logging::error(format!("保存便签位置失败：{error}"));
    }
}

/// 在 setup 中为 note 窗口注册拖动跟踪；只有 remember 模式才记录。
fn register_note_move_tracking(app: &AppHandle, stored: &StoredConfig) {
    cache_note_position_mode(&stored.window.position_mode);

    let Some(note) = app.get_webview_window("note") else {
        return;
    };
    let handle = app.clone();
    note.on_window_event(move |event| {
        if let tauri::WindowEvent::Moved(position) = event {
            if note_remembers_position() {
                schedule_note_position_save(handle.clone(), position.x, position.y);
            }
        }
    });
}

// ---------- 选区直读（SPEC 4.1） ----------

/// 读剪贴板文本：Windows 优先用原生 `CF_UNICODETEXT`（可判断“原本是否文本”），
/// 其它平台或原生读不到时回退剪贴板插件。
fn clipboard_read_text(app: &AppHandle) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        if let Some(text) = read_clipboard_text_native() {
            return Some(text);
        }
    }
    app.clipboard().read_text().ok()
}

fn source_text_result(
    text: Option<String>,
    from_selection: bool,
    clipboard_restored: bool,
) -> Result<SourceText, String> {
    match text {
        Some(text) if !text.trim().is_empty() => Ok(SourceText {
            text,
            from_selection,
            clipboard_restored,
        }),
        _ => Err(EMPTY_CLIPBOARD_ERROR.to_string()),
    }
}

#[tauri::command(async)]
fn read_source_text(app: AppHandle) -> Result<SourceText, String> {
    let stored = load_stored_config(&app);
    if !stored.behavior.read_selection {
        return source_text_result(clipboard_read_text(&app), false, false);
    }

    #[cfg(target_os = "windows")]
    {
        if foreground_is_own_process() {
            return source_text_result(clipboard_read_text(&app), false, false);
        }
        // 热键的修饰键可能还没松开：此时直接发 Ctrl+C 会变成 Ctrl+Alt+C，
        // 目标窗口不会复制。先等修饰键抬起（最多 120ms）再模拟。
        wait_for_modifiers_released();
        let snapshot = clipboard_read_text(&app);
        read_selection_on_windows(&app, snapshot)
    }

    #[cfg(not(target_os = "windows"))]
    {
        // macOS：本轮不模拟选区复制，行为与 0.3 一致直接读剪贴板。
        source_text_result(clipboard_read_text(&app), false, false)
    }
}

#[cfg(target_os = "windows")]
const CF_UNICODETEXT_FORMAT: u32 = 13;

#[cfg(target_os = "windows")]
fn foreground_is_own_process() -> bool {
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };

    unsafe {
        let foreground = GetForegroundWindow();
        if foreground.is_null() {
            return false;
        }
        let mut process_id: u32 = 0;
        GetWindowThreadProcessId(foreground, &mut process_id);
        process_id != 0 && process_id == GetCurrentProcessId()
    }
}

#[cfg(target_os = "windows")]
fn wait_for_modifiers_released() {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };

    const MAX_WAIT: Duration = Duration::from_millis(120);
    let started = Instant::now();
    loop {
        let pressed = [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
            .iter()
            .any(|key| (unsafe { GetAsyncKeyState(*key as i32) } as u16 & 0x8000) != 0);
        if !pressed || started.elapsed() >= MAX_WAIT {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(target_os = "windows")]
fn read_selection_on_windows(
    app: &AppHandle,
    snapshot: Option<String>,
) -> Result<SourceText, String> {
    use windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        keybd_event, KEYEVENTF_KEYUP, VK_C, VK_CONTROL,
    };

    let sequence = unsafe { GetClipboardSequenceNumber() };
    // 原剪贴板里有图片/HTML/RTF 等非文本格式时，恢复只会退回纯文本部分，
    // 按 FR-16 记一条 degraded 日志（不弹窗）。
    let degraded_restore = clipboard_has_non_text_formats();
    unsafe {
        keybd_event(VK_CONTROL as u8, 0, 0, 0);
        keybd_event(VK_C as u8, 0, 0, 0);
        keybd_event(VK_C as u8, 0, KEYEVENTF_KEYUP, 0);
        keybd_event(VK_CONTROL as u8, 0, KEYEVENTF_KEYUP, 0);
    }

    // 20ms 步长轮询，最长 200ms；命中后立刻读取新文本。
    let mut changed = false;
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(20));
        if unsafe { GetClipboardSequenceNumber() } != sequence {
            changed = true;
            break;
        }
    }

    let current = clipboard_read_text(app);
    if changed {
        if let Some(text) = current.as_deref().filter(|text| !text.trim().is_empty()) {
            let owner = note_window_hwnd(app);
            let restored = match snapshot.as_deref() {
                Some(previous) => {
                    let restored = write_clipboard_text_native(previous, owner);
                    if !restored {
                        logging::error("剪贴板恢复失败，保留选区文本");
                    } else if degraded_restore {
                        logging::error("clipboard restore degraded");
                    }
                    restored
                }
                None => {
                    logging::info("剪贴板原内容不是文本或读取失败，未恢复");
                    false
                }
            };
            return Ok(SourceText {
                text: text.to_string(),
                from_selection: true,
                clipboard_restored: restored,
            });
        }

        // 复制动作已经改了剪贴板，但选区为空/纯空白：先把原内容恢复回去，
        // 再回退去读剪贴板（此时读到的就是快照），不能让用户剪贴板被覆盖。
        if let Some(previous) = snapshot.as_deref() {
            let owner = note_window_hwnd(app);
            if !write_clipboard_text_native(previous, owner) {
                logging::error("选区为空且剪贴板恢复失败");
            }
        }
        return source_text_result(snapshot, false, false);
    }

    if !changed {
        // 超时：剪贴板仍应是快照；读取失败时用快照兜底。
        return source_text_result(current.or(snapshot), false, false);
    }
    source_text_result(current, false, false)
}

/// 取一个本进程窗口句柄：`OpenClipboard(NULL)` 会让 `SetClipboardData` 失败，
/// 恢复剪贴板时必须关联到真实窗口（SPEC 4.1 的 `clipboard_restored`）。
#[cfg(target_os = "windows")]
fn note_window_hwnd(app: &AppHandle) -> Option<windows_sys::Win32::Foundation::HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let window = app.get_webview_window("note")?;
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as *mut _),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
fn open_clipboard_retry(owner: Option<windows_sys::Win32::Foundation::HWND>) -> bool {
    use windows_sys::Win32::System::DataExchange::OpenClipboard;

    let owner = owner.unwrap_or(std::ptr::null_mut());
    for attempt in 0..3 {
        if unsafe { OpenClipboard(owner) } != 0 {
            return true;
        }
        if attempt < 2 {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    false
}

#[cfg(target_os = "windows")]
fn read_clipboard_text_native() -> Option<String> {
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, GetClipboardData, IsClipboardFormatAvailable,
    };
    use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT_FORMAT) == 0 {
            return None;
        }
        if !open_clipboard_retry(None) {
            return None;
        }
        let mut text = None;
        let handle = GetClipboardData(CF_UNICODETEXT_FORMAT);
        if !handle.is_null() {
            let size = GlobalSize(handle);
            if size >= 2 {
                let pointer = GlobalLock(handle);
                if !pointer.is_null() {
                    let units = std::slice::from_raw_parts(pointer as *const u16, size / 2);
                    let length = units
                        .iter()
                        .position(|unit| *unit == 0)
                        .unwrap_or(units.len());
                    text = Some(String::from_utf16_lossy(&units[..length]));
                    GlobalUnlock(handle);
                }
            }
        }
        CloseClipboard();
        text
    }
}

#[cfg(target_os = "windows")]
fn clipboard_has_non_text_formats() -> bool {
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, EnumClipboardFormats};

    // CF_TEXT(1) / CF_OEMTEXT(7) / CF_UNICODETEXT(13) / CF_LOCALE(16) 视作纯文本。
    unsafe {
        if !open_clipboard_retry(None) {
            return false;
        }
        let mut format = EnumClipboardFormats(0);
        let mut non_text = false;
        while format != 0 {
            if !matches!(format, 1 | 7 | 13 | 16) {
                non_text = true;
                break;
            }
            format = EnumClipboardFormats(format);
        }
        CloseClipboard();
        non_text
    }
}

#[cfg(target_os = "windows")]
fn write_clipboard_text_native(
    text: &str,
    owner: Option<windows_sys::Win32::Foundation::HWND>,
) -> bool {
    use windows_sys::Win32::Foundation::GlobalFree;
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
    };

    let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = units.len() * std::mem::size_of::<u16>();

    unsafe {
        let handle = GlobalAlloc(GMEM_MOVEABLE, bytes);
        if handle.is_null() {
            return false;
        }
        let pointer = GlobalLock(handle);
        if pointer.is_null() {
            GlobalFree(handle);
            return false;
        }
        std::ptr::copy_nonoverlapping(units.as_ptr() as *const u8, pointer as *mut u8, bytes);
        GlobalUnlock(handle);

        if !open_clipboard_retry(owner) {
            GlobalFree(handle);
            return false;
        }
        let cleared = EmptyClipboard() != 0;
        let owned = cleared && !SetClipboardData(CF_UNICODETEXT_FORMAT, handle).is_null();
        CloseClipboard();
        if !owned {
            GlobalFree(handle);
        }
        owned
    }
}

// ---------- 翻译 ----------

async fn run_translation(
    app: AppHandle,
    config: RuntimeConfig,
    text: String,
    counter: Arc<AtomicU64>,
    request_id: u64,
) -> Result<Option<String>, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))?;

    let body = json!({
        "model": config.model,
        "messages": [
            { "role": "system", "content": config.prompt },
            { "role": "user", "content": format!("<source_text>\n{text}\n</source_text>") }
        ],
        "temperature": 0.2,
        "stream": true
    });

    let response = client
        .post(normalize_url(&config.base_url))
        .bearer_auth(&config.api_key)
        .header("Accept", "text/event-stream")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("无法连接翻译服务：{e}"))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(friendly_http_error(status.as_u16(), &body));
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if !content_type.contains("text/event-stream") {
        let value: Value = response
            .json()
            .await
            .map_err(|e| format!("解析响应失败：{e}"))?;
        let content = value
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if content.is_empty() {
            return Err("模型未返回译文，请重试".into());
        }
        if counter.load(Ordering::SeqCst) != request_id {
            return Ok(None);
        }
        let _ = app.emit_to("note", "translation-chunk", content);
        return Ok(Some(content.to_string()));
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut got_content = false;
    let mut translation = String::new();

    while let Some(chunk) = stream.next().await {
        if counter.load(Ordering::SeqCst) != request_id {
            return Ok(None);
        }
        let chunk = chunk.map_err(|e| format!("读取流失败：{e}"))?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buffer.find('\n') {
            let mut line = buffer[..pos].to_string();
            buffer.drain(..=pos);
            line = line.trim_end_matches('\r').to_string();
            if line.is_empty() {
                continue;
            }
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data == "[DONE]" {
                if !got_content {
                    return Err("模型未返回译文，请重试".into());
                }
                return Ok(Some(translation));
            }
            let Ok(value) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            if let Some(delta) = value
                .pointer("/choices/0/delta/content")
                .and_then(|v| v.as_str())
            {
                if !delta.is_empty() {
                    got_content = true;
                    if counter.load(Ordering::SeqCst) != request_id {
                        return Ok(None);
                    }
                    translation.push_str(delta);
                    let _ = app.emit_to("note", "translation-chunk", delta);
                }
            }
        }
    }

    if !got_content {
        return Err("模型未返回译文，请重试".into());
    }
    Ok(Some(translation))
}

#[tauri::command]
fn start_translation(
    app: AppHandle,
    state: State<'_, TranslationState>,
    text: String,
) -> Result<(), String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("剪贴板没有可翻译的文本".into());
    }

    let stored = load_stored_config(&app);
    let max_chars = if stored.behavior.max_chars == 0 {
        DEFAULT_MAX_CHARS
    } else {
        stored.behavior.max_chars
    };
    if text.chars().count() > max_chars as usize {
        return Err(format!("内容过长（超过 {max_chars} 字符）"));
    }

    let api_key = load_api_key()?.ok_or_else(|| "尚未配置 API Key，请先打开设置。".to_string())?;
    let prompt = resolve_prompt(&stored.prompt.system, &stored.target_language);
    let config = RuntimeConfig {
        base_url: stored.base_url.clone(),
        model: stored.model.clone(),
        target_language: stored.target_language.clone(),
        api_key,
        prompt,
    };
    let history = stored.history.clone();
    let history_source = text.clone();

    let counter = state.0.clone();
    let request_id = counter.fetch_add(1, Ordering::SeqCst) + 1;
    let app_for_task = app.clone();

    logging::info(format!(
        "translation started: request={request_id} chars={}",
        text.chars().count()
    ));

    tauri::async_runtime::spawn(async move {
        let started = Instant::now();
        let result =
            run_translation(app_for_task.clone(), config.clone(), text, counter.clone(), request_id)
                .await;

        if counter.load(Ordering::SeqCst) != request_id {
            return;
        }

        match result {
            Ok(Some(translation)) => {
                let duration_ms = started.elapsed().as_millis() as u64;
                // 最近翻译：仅成功且非空入列，不受历史开关影响（SPEC 4.5）。
                if !translation.trim().is_empty() {
                    push_recent(&translation);
                    rebuild_tray_menu_async(&app_for_task);
                }
                if history.enabled && !translation.is_empty() {
                    let now = chrono::Local::now();
                    let entry = HistoryEntry {
                        id: now.timestamp_millis() as u64,
                        time: now.to_rfc3339(),
                        source: history_source,
                        translation,
                        target_language: config.target_language.clone(),
                        model: config.model.clone(),
                        duration_ms,
                    };
                    if let Err(error) = append_history(&app_for_task, entry, history.limit) {
                        logging::error(format!("写入历史失败：{error}"));
                    }
                }
                logging::info(format!("translation done in {duration_ms} ms"));
                let _ = app_for_task.emit_to("note", "translation-done", ());
            }
            Ok(None) => {
                logging::info("translation cancelled");
            }
            Err(error) => {
                logging::error(format!("translation failed: {error}"));
                let _ = app_for_task.emit_to("note", "translation-error", error);
            }
        }
    });

    Ok(())
}

#[tauri::command]
async fn test_connection(config: ConfigInput) -> Result<String, String> {
    let api_key = match config
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        Some(key) => key.to_string(),
        None => load_api_key()?.ok_or_else(|| "请先填写 API Key".to_string())?,
    };

    let started = Instant::now();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败：{e}"))?;
    let body = json!({
        "model": config.model,
        "messages": [
            { "role": "system", "content": "只输出译文，不要解释。" },
            { "role": "user", "content": "Hello, world." }
        ],
        "temperature": 0.2,
        "stream": false
    });
    let response = client
        .post(normalize_url(&config.base_url))
        .bearer_auth(&api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("连接失败：{e}"))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(friendly_http_error(status.as_u16(), &text));
    }
    let value: Value = serde_json::from_str(&text).map_err(|e| format!("解析响应失败：{e}"))?;
    let content = value
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .unwrap_or("(无内容)");
    Ok(format!(
        "连接成功（{} ms）：{}",
        started.elapsed().as_millis(),
        content
    ))
}

// ---------- 历史 ----------

fn read_history_file(path: &Path) -> Vec<HistoryEntry> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    match serde_json::from_str::<Vec<HistoryEntry>>(&text) {
        Ok(entries) => entries,
        Err(error) => {
            logging::error(format!("历史文件解析失败：{error}"));
            let bad = path.with_file_name("history.bad.json");
            let _ = std::fs::copy(path, &bad);
            Vec::new()
        }
    }
}

fn load_history(app: &AppHandle) -> Vec<HistoryEntry> {
    match history_path(app) {
        Ok(path) => read_history_file(&path),
        Err(_) => Vec::new(),
    }
}

fn save_history(app: &AppHandle, entries: &[HistoryEntry]) -> Result<(), String> {
    let path = history_path(app)?;
    let text = serde_json::to_string_pretty(entries).map_err(|e| e.to_string())?;
    write_atomic(&path, &text).map_err(|e| format!("保存历史失败：{e}"))
}

fn append_history(
    app: &AppHandle,
    entry: HistoryEntry,
    limit: u32,
) -> Result<(), String> {
    let mut entries = load_history(app);
    push_history(&mut entries, entry, limit);
    save_history(app, &entries)
}

#[tauri::command]
fn get_history(app: AppHandle) -> Vec<HistoryEntry> {
    load_history(&app)
}

#[tauri::command]
fn delete_history(app: AppHandle, id: u64) -> Result<Vec<HistoryEntry>, String> {
    let mut entries = load_history(&app);
    entries.retain(|entry| entry.id != id);
    save_history(&app, &entries)?;
    Ok(entries)
}

#[tauri::command]
fn clear_history(app: AppHandle) -> Result<Vec<HistoryEntry>, String> {
    save_history(&app, &[])?;
    logging::info("history cleared");
    Ok(Vec::new())
}

// ---------- 托盘与开机自启 ----------

fn set_pause_check(app: &AppHandle, paused: bool) {
    if let Ok(guard) = app.state::<TrayState>().menu.lock() {
        if let Some(menu) = guard.as_ref() {
            let _ = menu.pause.set_checked(paused);
        }
    }
}

fn update_pause_check(app: &AppHandle, paused: bool) {
    set_pause_check(app, paused);
}

fn set_autostart_check(app: &AppHandle, enabled: bool) {
    if let Ok(guard) = app.state::<TrayState>().menu.lock() {
        if let Some(menu) = guard.as_ref() {
            let _ = menu.autostart.set_checked(enabled);
        }
    }
}

#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, String> {
    let manager = app.autolaunch();
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    result.map_err(|e| format!("设置开机自启失败：{e}"))?;
    set_autostart_check(&app, enabled);
    let _ = app.emit_to("settings", "autostart-changed", enabled);
    logging::info(format!("autostart set to {enabled}"));
    Ok(enabled)
}

fn toggle_autostart(app: &AppHandle) {
    let current = app.autolaunch().is_enabled().unwrap_or(false);
    let next = !current;
    let manager = app.autolaunch();
    let result = if next {
        manager.enable()
    } else {
        manager.disable()
    };
    match result {
        Ok(()) => {
            set_autostart_check(app, next);
            let _ = app.emit_to("settings", "autostart-changed", next);
            logging::info(format!("autostart toggled to {next}"));
        }
        Err(error) => {
            logging::error(format!("autostart toggle failed: {error}"));
            set_autostart_check(app, current);
        }
    }
}

fn toggle_hotkey_pause(app: &AppHandle) {
    let paused = app
        .state::<HotkeyState>()
        .0
        .lock()
        .map(|inner| inner.paused)
        .unwrap_or(false);
    if let Err(error) = apply_hotkey_paused(app, !paused) {
        logging::error(format!("hotkey pause toggle failed: {error}"));
        set_pause_check(app, paused);
    }
}

// ---------- 最近翻译（SPEC 4.5） ----------

fn recent_snapshot() -> Vec<RecentEntry> {
    RECENT
        .lock()
        .map(|entries| entries.iter().cloned().collect())
        .unwrap_or_default()
}

fn push_recent(text: &str) {
    let entry = RecentEntry {
        at: chrono::Local::now().format("%H:%M").to_string(),
        text: text.to_string(),
    };
    match RECENT.lock() {
        Ok(mut entries) => {
            entries.push_front(entry);
            while entries.len() > RECENT_CAPACITY {
                entries.pop_back();
            }
        }
        Err(_) => logging::error("最近翻译记录加锁失败"),
    }
}

#[tauri::command]
fn get_recent_translations() -> Vec<RecentEntry> {
    recent_snapshot()
}

fn tray_handle(app: &AppHandle) -> Option<TrayIcon<Wry>> {
    app.state::<TrayState>()
        .tray
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
}

fn store_tray_menu(app: &AppHandle, pause: CheckMenuItem<Wry>, autostart: CheckMenuItem<Wry>) {
    if let Ok(mut guard) = app.state::<TrayState>().menu.lock() {
        *guard = Some(TrayMenu { pause, autostart });
    }
}

fn tray_menu_error(error: tauri::Error) -> String {
    format!("构建托盘菜单失败：{error}")
}

fn hotkey_paused(app: &AppHandle) -> bool {
    app.state::<HotkeyState>()
        .0
        .lock()
        .map(|inner| inner.paused)
        .unwrap_or(false)
}

/// 「最近翻译」子菜单：无记录时只有一条禁用项（SPEC 4.5）。
fn build_recent_submenu(app: &AppHandle) -> Result<Submenu<Wry>, String> {
    let entries = recent_snapshot();
    if entries.is_empty() {
        let empty = MenuItem::with_id(app, "recent-empty", "（暂无记录）", false, None::<&str>)
            .map_err(tray_menu_error)?;
        return Submenu::with_items(app, "最近翻译", true, &[&empty]).map_err(tray_menu_error);
    }

    let mut items = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let label = format_recent_label(&entry.at, &entry.text);
        items.push(
            MenuItem::with_id(app, format!("recent:{index}"), label, true, None::<&str>)
                .map_err(tray_menu_error)?,
        );
    }
    let refs: Vec<&dyn IsMenuItem<Wry>> = items
        .iter()
        .map(|item| item as &dyn IsMenuItem<Wry>)
        .collect();
    Submenu::with_items(app, "最近翻译", true, &refs).map_err(tray_menu_error)
}

struct BuiltTrayMenu {
    menu: Menu<Wry>,
    pause: CheckMenuItem<Wry>,
    autostart: CheckMenuItem<Wry>,
}

fn build_tray_menu(app: &AppHandle) -> Result<BuiltTrayMenu, String> {
    let paused = hotkey_paused(app);
    let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);

    let translate = MenuItem::with_id(app, "translate", "翻译剪贴板", true, None::<&str>)
        .map_err(tray_menu_error)?;
    let recent = build_recent_submenu(app)?;
    let settings =
        MenuItem::with_id(app, "settings", "设置", true, None::<&str>).map_err(tray_menu_error)?;
    let pause =
        CheckMenuItem::with_id(app, "toggle-pause", "暂停热键", true, paused, None::<&str>)
            .map_err(tray_menu_error)?;
    let autostart = CheckMenuItem::with_id(
        app,
        "toggle-autostart",
        "开机自动启动",
        true,
        autostart_enabled,
        None::<&str>,
    )
    .map_err(tray_menu_error)?;
    let separator_one = PredefinedMenuItem::separator(app).map_err(tray_menu_error)?;
    let separator_two = PredefinedMenuItem::separator(app).map_err(tray_menu_error)?;
    let quit =
        MenuItem::with_id(app, "quit", "退出", true, None::<&str>).map_err(tray_menu_error)?;
    let menu = Menu::with_items(
        app,
        &[
            &translate,
            &recent,
            &settings,
            &separator_one,
            &pause,
            &autostart,
            &separator_two,
            &quit,
        ],
    )
    .map_err(tray_menu_error)?;

    Ok(BuiltTrayMenu {
        menu,
        pause,
        autostart,
    })
}

/// 翻译成功后重建托盘菜单；`set_menu` 必须在主线程执行（SPEC 4.5）。
fn rebuild_tray_menu_async(app: &AppHandle) {
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        if let Err(error) = rebuild_tray_menu(&handle) {
            logging::error(format!("重建最近翻译菜单失败：{error}"));
        }
    }) {
        logging::error(format!("调度托盘菜单重建失败：{error}"));
    }
}

fn rebuild_tray_menu(app: &AppHandle) -> Result<(), String> {
    let tray = tray_handle(app).ok_or_else(|| "托盘尚未初始化".to_string())?;
    let built = build_tray_menu(app)?;
    tray.set_menu(Some(built.menu))
        .map_err(|error| format!("更新托盘菜单失败：{error}"))?;
    store_tray_menu(app, built.pause, built.autostart);
    Ok(())
}

fn set_tray_tooltip(app: &AppHandle, tooltip: Option<&str>) {
    if let Some(tray) = tray_handle(app) {
        let _ = tray.set_tooltip(tooltip);
    }
}

/// 点击最近翻译后把 tooltip 改成「已复制：…」，3 秒后恢复默认（无 tooltip）。
fn flash_tray_tooltip(app: &AppHandle, message: String) {
    let generation = TRAY_TOOLTIP_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    set_tray_tooltip(app, Some(&message));
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(TRAY_TOOLTIP_DURATION);
        if TRAY_TOOLTIP_GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        let restore = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            if TRAY_TOOLTIP_GENERATION.load(Ordering::SeqCst) == generation {
                set_tray_tooltip(&restore, None);
            }
        });
    });
}

fn copy_recent_entry(app: &AppHandle, index: usize) {
    let Some(entry) = recent_snapshot().get(index).cloned() else {
        return;
    };
    match app.clipboard().write_text(entry.text.clone()) {
        Ok(()) => {
            let preview = preview_text(&entry.text);
            flash_tray_tooltip(app, format!("已复制：{preview}"));
            logging::info(format!("recent translation copied: index={index}"));
        }
        Err(error) => logging::error(format!("复制最近翻译失败：{error}")),
    }
}

fn handle_tray_menu_event(app: &AppHandle, event: MenuEvent) {
    let id = event.id.as_ref();
    if let Some(index) = id.strip_prefix("recent:") {
        if let Ok(index) = index.parse::<usize>() {
            copy_recent_entry(app, index);
        }
        return;
    }

    match id {
        "translate" => {
            // 取点期间忽略「翻译剪贴板」（SPEC 4.4）。
            if PICKING.load(Ordering::SeqCst) {
                return;
            }
            let _ = app.emit_to("note", "hotkey-pressed", ());
        }
        "settings" => {
            if let Err(error) = open_settings(app.clone()) {
                logging::error(format!("open settings from tray failed: {error}"));
            }
        }
        "toggle-pause" => toggle_hotkey_pause(app),
        "toggle-autostart" => toggle_autostart(app),
        "quit" => app.exit(0),
        _ => {}
    }
}

fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let Some(icon) = app.default_window_icon().cloned() else {
        return Ok(());
    };

    let handle = app.handle();
    let built = build_tray_menu(handle)?;

    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .menu(&built.menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| handle_tray_menu_event(app, event))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if let Err(error) = open_settings(tray.app_handle().clone()) {
                    logging::error(format!("open settings from tray click failed: {error}"));
                }
            }
        })
        .build(app)?;

    if let Ok(mut guard) = app.state::<TrayState>().tray.lock() {
        *guard = Some(tray);
    }
    store_tray_menu(handle, built.pause, built.autostart);

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Err(error) = open_settings(app.clone()) {
                logging::error(format!("single instance focus failed: {error}"));
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        // 取点期间屏蔽全局热键（SPEC 4.4）。
                        if PICKING.load(Ordering::SeqCst) {
                            return;
                        }
                        let _ = app.emit_to("note", "hotkey-pressed", ());
                    }
                })
                .build(),
        )
        .manage(TranslationState::default())
        .manage(HotkeyState::default())
        .manage(TrayState::default())
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            dismiss_config_warning,
            get_app_info,
            export_config,
            import_config,
            reset_config,
            get_hotkey_status,
            set_hotkey,
            set_hotkey_paused,
            get_autostart,
            set_autostart,
            get_history,
            delete_history,
            clear_history,
            get_recent_translations,
            open_settings,
            close_settings,
            quit_app,
            show_note,
            hide_note,
            read_source_text,
            set_note_size,
            set_note_pinned,
            begin_position_pick,
            cancel_position_pick,
            confirm_position_pick,
            start_translation,
            test_connection,
            check_update,
            open_release_page
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let stored = load_stored_config(&handle);
            configure_logging(&handle, &stored);
            logging::info(format!(
                "LingoNote {} starting",
                env!("CARGO_PKG_VERSION")
            ));
            logging::info(format!(
                "update auto-check enabled: {}",
                stored.behavior.check_updates
            ));

            register_initial_hotkey(&handle, &stored);
            register_note_move_tracking(&handle, &stored);

            // 恢复上次拖拽保存的便签尺寸（AC-28）；窗口在 tauri.conf 里是固定的 420×360。
            if let Some(note) = app.get_webview_window("note") {
                let (width, height) =
                    clamp_note_size(stored.window.note_width, stored.window.note_height);
                if let Err(error) = note.set_size(LogicalSize::new(width, height)) {
                    logging::error(format!("恢复便签尺寸失败：{error}"));
                }
            }

            let _ = setup_tray(app);

            if let Some(settings) = app.get_webview_window("settings") {
                let settings_for_event = settings.clone();
                settings.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = settings_for_event.hide();
                    }
                });

                let has_warning = CONFIG_WARNING
                    .lock()
                    .map(|warning| warning.is_some())
                    .unwrap_or(false);
                if load_api_key().ok().flatten().is_none() || has_warning {
                    let _ = settings.show();
                    let _ = settings.set_focus();
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
