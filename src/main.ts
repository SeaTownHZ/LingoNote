import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import "./note.css";

interface UiConfig {
  background: string;
  foreground: string;
  font_family: string;
  font_size: number;
  line_height: number;
  radius: number;
  opacity: number;
}

interface WindowConfig {
  position_mode: string;
  anchor: string;
  margin_x: number;
  margin_y: number;
  last_x: number | null;
  last_y: number | null;
  /** 便签宽高（CSS 像素；Rust 侧再 clamp 320–900 / 220–900）。 */
  note_width: number;
  note_height: number;
  /** 钉住后忽略「失焦隐藏」。 */
  pinned: boolean;
}

interface BehaviorConfig {
  hide_on_blur: boolean;
  copy_restores_clipboard: boolean;
  appear_focused: boolean;
  pause_hotkey: boolean;
  max_chars: number;
  /** FR-16 选区直读：实际读取由 Rust 完成，前端仅同步状态。 */
  read_selection: boolean;
  /** FR-18 连按热键＝复制译文并收起便签。 */
  double_press_copies: boolean;
}

interface ConfigView {
  schema_version: number;
  base_url: string;
  model: string;
  target_language: string;
  hotkey: string;
  ui: UiConfig;
  behavior: BehaviorConfig;
  window: WindowConfig;
  prompt: string;
  default_prompt: string;
  history: { enabled: boolean; limit: number };
  logging: { enabled: boolean; level: string };
  has_api_key: boolean;
  warning: string | null;
}

interface SourceText {
  text: string;
  from_selection: boolean;
  clipboard_restored: boolean;
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
const pinBtn = el<HTMLButtonElement>("pin");
const closeBtn = el<HTMLButtonElement>("close");
const langEl = el<HTMLSpanElement>("lang");
const resizeHandle = el<HTMLDivElement>("resize");
const toastEl = el<HTMLDivElement>("toast");

/** 便签尺寸范围（CSS 像素，与后端 clamp 一致）。 */
const MIN_NOTE_WIDTH = 320;
const MIN_NOTE_HEIGHT = 220;
const MAX_NOTE_WIDTH = 900;
const MAX_NOTE_HEIGHT = 900;

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
/** FR-16：开关状态同步（真正的选区读取在 Rust 的 `read_source_text` 里）。 */
let readSelection = true;
/** FR-18：连按热键复制译文并收起便签。 */
let doublePressCopies = true;
/** FR-19 C：钉住后忽略失焦隐藏。 */
let pinned = false;
let noteWidth = 420;
let noteHeight = 360;
/** FR-19 A：尺寸把手拖动状态；null 表示当前没有在拖动。 */
let resizeOrigin: {
  pointerId: number;
  screenX: number;
  screenY: number;
  width: number;
  height: number;
} | null = null;
let sizeFrame = 0;
let pendingSize: { width: number; height: number } | null = null;

function setStatus(text: string) {
  statusEl.textContent = text;
}

let toastTimer: number | undefined;
/** 「复制译文后 1 秒恢复剪贴板」的定时器；连按复制/新翻译/关闭时必须取消。 */
let copyRestoreTimer: number | undefined;

function cancelCopyRestore() {
  if (copyRestoreTimer !== undefined) {
    window.clearTimeout(copyRestoreTimer);
    copyRestoreTimer = undefined;
  }
}

/** 轻提示：默认 1.5 秒后自动隐藏，重复调用会重置计时。 */
function showToast(text: string, ms = 2000) {
  toastEl.textContent = text;
  toastEl.classList.remove("hidden");
  if (toastTimer !== undefined) window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => {
    toastTimer = undefined;
    toastEl.classList.add("hidden");
  }, ms);
}

function hideToast() {
  if (toastTimer !== undefined) {
    window.clearTimeout(toastTimer);
    toastTimer = undefined;
  }
  toastEl.classList.add("hidden");
}

/** FR-19 C：同步图钉按钮的外观与标题。 */
function syncPin() {
  pinBtn.classList.toggle("active", pinned);
  pinBtn.title = pinned ? "取消钉住" : "钉住";
  pinBtn.setAttribute("aria-pressed", pinned ? "true" : "false");
}

function clampNoteSize(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Math.round(value)));
}

function persistNoteSize(width: number, height: number) {
  void invoke("set_note_size", { width, height }).catch((error) => {
    console.error("set_note_size failed", error);
  });
}

/** 拖动中按帧节流更新窗口尺寸，避免每来一个 pointermove 就打一次 IPC。 */
function applyNoteSize(width: number, height: number) {
  noteWidth = width;
  noteHeight = height;
  pendingSize = { width, height };
  if (sizeFrame) return;
  sizeFrame = window.requestAnimationFrame(() => {
    sizeFrame = 0;
    const size = pendingSize;
    pendingSize = null;
    if (!size) return;
    persistNoteSize(size.width, size.height);
  });
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
  // 清空累积译文：错误态下连按热键不应复制半截内容（T5 P1）。
  translation = "";
  errorEl.textContent = message;
  errorEl.classList.remove("hidden");
  retryBtn.classList.remove("hidden");
  setStatus("出错了");
}

function resetView(text: string) {
  cancelCopyRestore();
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
  // 不在这里取消「复制后恢复」定时器：失焦隐藏是「复制完去别处粘贴」的正常路径。
  noteVisible = false;
  hasBeenFocused = false;
  hideToast();
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

/**
 * FR-18：连按热键＝复制译文 + 收起便签。
 * 这条路径**不触发** `copy_restores_clipboard` 的「1 秒后恢复原剪贴板」逻辑。
 */
async function copyTranslationAndClose() {
  cancelCopyRestore();
  try {
    await writeText(translation);
    showToast("已复制译文");
  } catch (error) {
    console.error("double press copy failed", error);
    showToast("复制失败");
  }
  window.setTimeout(() => {
    void hideNote();
  }, 400);
}

async function translateClipboard() {
  if (noteVisible) {
    // 有译文且开关打开 → 复制并关闭；否则回到 v0.3：只隐藏（流式中/错误态同理）。
    if (doublePressCopies && !loading && translation.trim()) {
      await copyTranslationAndClose();
      return;
    }
    await hideNote();
    return;
  }

  let text = "";
  let fromSelection = false;
  try {
    // FR-16：优先读当前选区（不破坏剪贴板），失败/无选区时 Rust 回退剪贴板。
    const source = await invoke<SourceText>("read_source_text");
    text = source.text;
    fromSelection = source.from_selection === true;
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

  // 选区直读时 Rust 已把剪贴板恢复成热键前内容，前端不再承诺「复制后 1 秒恢复」，
  // 否则会用原文覆盖掉用户原本的剪贴板。
  originalClipboard = fromSelection ? "" : text;
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
  // 旧配置没有新字段时按契约默认值（true）处理。
  doublePressCopies = config.behavior.double_press_copies !== false;
  readSelection = config.behavior.read_selection !== false;
  maxChars = config.behavior.max_chars || 20_000;
  hotkeyPaused = config.behavior.pause_hotkey;
  hotkeyText = config.hotkey || "Alt+Q";
  pinned = config.window?.pinned === true;
  syncPin();
  // 拖动过程中不要被 config-changed 覆盖本地尺寸。
  if (!resizeOrigin) {
    noteWidth = clampNoteSize(config.window?.note_width ?? 420, MIN_NOTE_WIDTH, MAX_NOTE_WIDTH);
    noteHeight = clampNoteSize(config.window?.note_height ?? 360, MIN_NOTE_HEIGHT, MAX_NOTE_HEIGHT);
  }
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
    // FR-19 C：钉住时忽略失焦隐藏。
    if (noteVisible && hasBeenFocused && hideOnBlur && !pinned) {
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

  // FR-19 C：图钉切换。
  pinBtn.addEventListener("click", async () => {
    const next = !pinned;
    try {
      await invoke("set_note_pinned", { pinned: next });
      pinned = next;
      syncPin();
    } catch (error) {
      console.error("set_note_pinned failed", error);
      showToast("钉住失败");
    }
  });

  // FR-19 A：右下角把手拖拽调整便签尺寸。
  resizeHandle.addEventListener("pointerdown", (event) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    resizeOrigin = {
      pointerId: event.pointerId,
      screenX: event.screenX,
      screenY: event.screenY,
      width: noteWidth,
      height: noteHeight,
    };
    document.body.classList.add("note-resizing");
    resizeHandle.setPointerCapture(event.pointerId);
  });

  window.addEventListener(
    "pointermove",
    (event) => {
      if (!resizeOrigin || event.pointerId !== resizeOrigin.pointerId) return;
      event.preventDefault();
      const width = clampNoteSize(
        resizeOrigin.width + (event.screenX - resizeOrigin.screenX),
        MIN_NOTE_WIDTH,
        MAX_NOTE_WIDTH,
      );
      const height = clampNoteSize(
        resizeOrigin.height + (event.screenY - resizeOrigin.screenY),
        MIN_NOTE_HEIGHT,
        MAX_NOTE_HEIGHT,
      );
      if (width === noteWidth && height === noteHeight) return;
      applyNoteSize(width, height);
    },
    { passive: false },
  );

  const endResize = (event: PointerEvent) => {
    if (!resizeOrigin || event.pointerId !== resizeOrigin.pointerId) return;
    resizeOrigin = null;
    document.body.classList.remove("note-resizing");
    if (resizeHandle.hasPointerCapture(event.pointerId)) {
      resizeHandle.releasePointerCapture(event.pointerId);
    }
    // 松手后补一次最终尺寸，保证下次打开时与松手时一致。
    persistNoteSize(noteWidth, noteHeight);
  };

  window.addEventListener("pointerup", endResize);
  window.addEventListener("pointercancel", endResize);

  copyBtn.addEventListener("click", async () => {
    if (!translation) return;
    try {
      await writeText(translation);
      showToast("已复制");
      if (copyRestoresClipboard && originalClipboard) {
        setStatus("已复制（1 秒后恢复原剪贴板）");
        cancelCopyRestore();
        copyRestoreTimer = window.setTimeout(() => {
          copyRestoreTimer = undefined;
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
