import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";

interface UiConfig {
  background: string;
  foreground: string;
  font_family: string;
  font_size: number;
  line_height: number;
  radius: number;
  opacity: number;
}

interface BehaviorConfig {
  hide_on_blur: boolean;
  copy_restores_clipboard: boolean;
  appear_focused: boolean;
  pause_hotkey: boolean;
  max_chars: number;
}

interface ConfigView {
  schema_version: number;
  base_url: string;
  model: string;
  target_language: string;
  hotkey: string;
  ui: UiConfig;
  behavior: BehaviorConfig;
  prompt: string;
  default_prompt: string;
  history: { enabled: boolean; limit: number };
  logging: { enabled: boolean; level: string };
  has_api_key: boolean;
  warning: string | null;
}

interface HotkeyStatus {
  hotkey: string;
  error: string | null;
}

const el = <T extends HTMLElement = HTMLElement>(id: string): T => {
  const node = document.getElementById(id);
  if (!node) throw new Error(`missing element #${id}`);
  return node as T;
};

const loadingEl = el<HTMLDivElement>("loading");
const translationEl = el<HTMLDivElement>("translation");
const errorEl = el<HTMLDivElement>("error");
const originalWrap = el<HTMLDivElement>("original-wrap");
const originalEl = el<HTMLDivElement>("original");
const statusEl = el<HTMLSpanElement>("status");
const metaEl = el<HTMLSpanElement>("meta");
const toggleOriginalBtn = el<HTMLButtonElement>("toggle-original");
const copyBtn = el<HTMLButtonElement>("copy");
const retryBtn = el<HTMLButtonElement>("retry");
const settingsBtn = el<HTMLButtonElement>("settings");
const closeBtn = el<HTMLButtonElement>("close");
const langEl = el<HTMLSpanElement>("lang");

let noteVisible = false;
let hasBeenFocused = false;
let lastText = "";
let translation = "";
let loading = false;
let startedAt = 0;
let hideOnBlur = true;
let copyRestoresClipboard = false;
let maxChars = 20_000;
let hotkeyPaused = false;
let hotkeyText = "Alt+Q";
let originalClipboard = "";
let currentConfig: ConfigView | null = null;

function setStatus(text: string) {
  statusEl.textContent = text;
}

/** 不透明度是 80–100 的整数百分比；缺失或非法时回落 100。 */
function normalizeOpacity(value: number | undefined): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return 100;
  return Math.min(100, Math.max(80, Math.round(value)));
}

function applyUi(ui: UiConfig) {
  const root = document.documentElement;
  root.style.setProperty("--note-bg", ui.background || "#FFFFFF");
  root.style.setProperty("--note-fg", ui.foreground || "#111111");
  root.style.setProperty("--note-font", ui.font_family || "SimSun");
  root.style.setProperty("--note-size", `${ui.font_size || 16}px`);
  root.style.setProperty("--note-line", `${ui.line_height || 1.65}`);
  root.style.setProperty("--note-radius", `${ui.radius ?? 12}px`);
  root.style.setProperty("--note-opacity", String(normalizeOpacity(ui.opacity) / 100));
}

function showLoading() {
  loading = true;
  loadingEl.classList.remove("hidden");
  errorEl.classList.add("hidden");
  translationEl.textContent = "";
  translation = "";
  retryBtn.classList.remove("hidden");
  setStatus("翻译中…");
}

function renderTranslation() {
  loadingEl.classList.add("hidden");
  errorEl.classList.add("hidden");
  translationEl.textContent = translation;
}

function showError(message: string) {
  loading = false;
  loadingEl.classList.add("hidden");
  translationEl.textContent = "";
  errorEl.textContent = message;
  errorEl.classList.remove("hidden");
  retryBtn.classList.remove("hidden");
  setStatus("出错了");
}

function resetView(text: string) {
  lastText = text;
  translation = "";
  originalEl.textContent = text;
  originalWrap.classList.add("hidden");
  toggleOriginalBtn.textContent = "原文";
  errorEl.classList.add("hidden");
  errorEl.textContent = "";
  startedAt = performance.now();
  metaEl.textContent = "";
  showLoading();
}

async function showNote() {
  try {
    await invoke("show_note");
  } catch (error) {
    console.error("show_note failed", error);
  }
  noteVisible = true;
  hasBeenFocused = false;
}

async function hideNote() {
  noteVisible = false;
  hasBeenFocused = false;
  try {
    await invoke("hide_note");
  } catch (error) {
    console.error("hide_note failed", error);
  }
}

async function startTranslation(text: string) {
  lastText = text;
  resetView(text);
  try {
    await invoke("start_translation", { text });
  } catch (error) {
    showError(String(error));
  }
}

async function translateClipboard() {
  if (noteVisible) {
    await hideNote();
    return;
  }

  let text = "";
  try {
    text = (await readText()) ?? "";
  } catch (error) {
    await showNote();
    showError(`无法读取剪贴板：${String(error)}`);
    return;
  }

  text = text.trim();
  if (!text) {
    await showNote();
    showError("剪贴板里没有可翻译的文本。复制一段文字后再按热键。");
    return;
  }
  if (text.length > maxChars) {
    await showNote();
    showError(`内容过长（${text.length} 字符，上限 ${maxChars}）。请分段复制或到设置里调整上限。`);
    return;
  }

  originalClipboard = text;
  await showNote();
  await startTranslation(text);
}

function finishTranslation() {
  if (!loading) return;
  loading = false;
  renderTranslation();
  if (!translation) {
    showError("模型未返回译文，请重试。");
    return;
  }
  const seconds = ((performance.now() - startedAt) / 1000).toFixed(1);
  setStatus("完成");
  metaEl.textContent = `${seconds}s`;
}

async function refreshConfig() {
  const config = await invoke<ConfigView>("get_config");
  currentConfig = config;
  langEl.textContent = config.target_language || "简中";
  hideOnBlur = config.behavior.hide_on_blur;
  copyRestoresClipboard = config.behavior.copy_restores_clipboard;
  maxChars = config.behavior.max_chars || 20_000;
  hotkeyPaused = config.behavior.pause_hotkey;
  hotkeyText = config.hotkey || "Alt+Q";
  applyUi(config.ui);
  if (!loading && errorEl.classList.contains("hidden")) {
    setStatus(
      hotkeyPaused
        ? "热键已暂停（托盘菜单仍可翻译）"
        : `${hotkeyText} 翻译剪贴板`,
    );
  }
}

async function checkHotkey() {
  const status = await invoke<HotkeyStatus>("get_hotkey_status");
  if (status.error) {
    await showNote();
    showError(status.error);
  }
}

async function setup() {
  await Promise.all([
    listen<string>("translation-chunk", (event) => {
      if (!loading) return;
      translation += event.payload;
      renderTranslation();
    }),
    listen("translation-done", () => {
      finishTranslation();
    }),
    listen<string>("translation-error", (event) => {
      showError(event.payload);
    }),
    listen("hotkey-pressed", () => {
      void translateClipboard();
    }),
    listen("config-changed", () => {
      void refreshConfig();
    }),
  ]);

  window.addEventListener("focus", () => {
    hasBeenFocused = true;
  });

  window.addEventListener("blur", () => {
    if (noteVisible && hasBeenFocused && hideOnBlur) {
      void hideNote();
    }
    hasBeenFocused = false;
  });

  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      void hideNote();
    }
  });

  copyBtn.addEventListener("click", async () => {
    if (!translation) return;
    try {
      await writeText(translation);
      if (copyRestoresClipboard && originalClipboard) {
        setStatus("已复制（1 秒后恢复原剪贴板）");
        window.setTimeout(() => {
          void writeText(originalClipboard)
            .then(() => {
              if (!loading) setStatus("完成");
            })
            .catch((error) => {
              console.error("restore clipboard failed", error);
            });
        }, 1000);
      } else {
        setStatus("已复制");
        window.setTimeout(() => {
          if (!loading) setStatus("完成");
        }, 1200);
      }
    } catch (error) {
      setStatus("复制失败");
      console.error(error);
    }
  });

  toggleOriginalBtn.addEventListener("click", () => {
    const hidden = originalWrap.classList.toggle("hidden");
    toggleOriginalBtn.textContent = hidden ? "原文" : "收起原文";
    if (!hidden) {
      originalWrap.scrollIntoView({ block: "nearest" });
    }
  });

  retryBtn.addEventListener("click", () => {
    if (lastText) {
      void startTranslation(lastText);
    }
  });

  settingsBtn.addEventListener("click", async () => {
    await hideNote();
    try {
      await invoke("open_settings");
    } catch (error) {
      await showNote();
      showError(`无法打开设置：${String(error)}`);
    }
  });

  closeBtn.addEventListener("click", () => {
    void hideNote();
  });

  await refreshConfig();

  // Rust setup 注册热键可能晚于本页 JS，稍等再读状态。
  window.setTimeout(() => {
    void checkHotkey().catch((error) => {
      console.error("check hotkey failed", error);
    });
  }, 800);
}

void setup().catch((error) => {
  console.error("setup failed", error);
});
