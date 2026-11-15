use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow, Wry};
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

mod logging;
mod pure;
use pure::{
    anchor_position, build_export, clamp_position, cursor_position, friendly_http_error,
    normalize_config, normalize_url, parse_import, push_history, resolve_prompt, write_atomic,
    ConfigInput, ConfigView, HistoryEntry, StoredConfig, WorkArea, CURSOR_GAP, DEFAULT_MAX_CHARS,
    POSITION_MODE_ANCHOR, POSITION_MODE_REMEMBER,
};

const KEYRING_SERVICE: &str = "LingoNote";
const KEYRING_USER: &str = "api-key";

static CONFIG_WARNING: Mutex<Option<String>> = Mutex::new(None);

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
struct TrayState(Mutex<Option<TrayMenu>>);

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
    let mut window = config.window;
    // 设置页自动保存不带 last_x/last_y（None），必须保留磁盘上记住的位置，
    // 否则每次改外观都会把“记住上次位置”清空。
    if window.last_x.is_none() {
        window.last_x = stored.window.last_x;
    }
    if window.last_y.is_none() {
        window.last_y = stored.window.last_y;
    }
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

            // 显示器选择：cursor/anchor 用光标所在屏；remember 用保存点最近的屏，
            // 保存点缺失时回退光标。
            let monitor_point = if window_config.position_mode == POSITION_MODE_REMEMBER {
                match (window_config.last_x, window_config.last_y) {
                    (Some(last_x), Some(last_y)) => POINT {
                        x: last_x,
                        y: last_y,
                    },
                    _ => POINT {
                        x: cursor_x,
                        y: cursor_y,
                    },
                }
            } else {
                POINT {
                    x: cursor_x,
                    y: cursor_y,
                }
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
    if let Ok(guard) = app.state::<TrayState>().0.lock() {
        if let Some(menu) = guard.as_ref() {
            let _ = menu.pause.set_checked(paused);
        }
    }
}

fn update_pause_check(app: &AppHandle, paused: bool) {
    set_pause_check(app, paused);
}

fn set_autostart_check(app: &AppHandle, enabled: bool) {
    if let Ok(guard) = app.state::<TrayState>().0.lock() {
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

fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let Some(icon) = app.default_window_icon().cloned() else {
        return Ok(());
    };

    let paused = app
        .state::<HotkeyState>()
        .0
        .lock()
        .map(|inner| inner.paused)
        .unwrap_or(false);
    let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);

    let translate = MenuItem::with_id(app, "translate", "翻译剪贴板", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(
        app,
        "toggle-pause",
        "暂停热键",
        true,
        paused,
        None::<&str>,
    )?;
    let autostart = CheckMenuItem::with_id(
        app,
        "toggle-autostart",
        "开机自动启动",
        true,
        autostart_enabled,
        None::<&str>,
    )?;
    let separator_one = PredefinedMenuItem::separator(app)?;
    let separator_two = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &translate,
            &settings,
            &separator_one,
            &pause,
            &autostart,
            &separator_two,
            &quit,
        ],
    )?;

    TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "translate" => {
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
        })
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

    if let Ok(mut guard) = app.state::<TrayState>().0.lock() {
        *guard = Some(TrayMenu { pause, autostart });
    }

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
            open_settings,
            close_settings,
            quit_app,
            show_note,
            hide_note,
            start_translation,
            test_connection
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let stored = load_stored_config(&handle);
            configure_logging(&handle, &stored);
            logging::info(format!(
                "LingoNote {} starting",
                env!("CARGO_PKG_VERSION")
            ));

            register_initial_hotkey(&handle, &stored);
            register_note_move_tracking(&handle, &stored);
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
