import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ask, open, save } from "@tauri-apps/plugin-dialog";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";

interface UiConfig {
  background: string;
  foreground: string;
  font_family: string;
  font_size: number;
  line_height: number;
  radius: number;
}

interface BehaviorConfig {
  hide_on_blur: boolean;
  copy_restores_clipboard: boolean;
  appear_focused: boolean;
  pause_hotkey: boolean;
  max_chars: number;
}

interface HistoryConfig {
  enabled: boolean;
  limit: number;
}

interface LoggingConfig {
  enabled: boolean;
  level: string;
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
  history: HistoryConfig;
  logging: LoggingConfig;
  has_api_key: boolean;
  warning: string | null;
}

interface ConfigInput {
  base_url: string;
  model: string;
  target_language: string;
  api_key: string | null;
  ui: UiConfig;
  behavior: BehaviorConfig;
  prompt: { system: string };
  history: HistoryConfig;
  logging: LoggingConfig;
}

interface HotkeyStatus {
  hotkey: string;
  error: string | null;
  paused: boolean;
}

interface HistoryEntry {
  id: number;
  time: string;
  source: string;
  translation: string;
  target_language: string;
  model: string;
  duration_ms: number;
}

interface AppInfo {
  version: string;
  identifier: string;
  config_dir: string;
  config_path: string;
  history_path: string;
  log_dir: string;
  hotkey_paused: boolean;
}

const el = <T extends HTMLElement = HTMLElement>(id: string): T => {
  const node = document.getElementById(id);
  if (!node) throw new Error(`missing element #${id}`);
  return node as T;
};

// 通用
const hotkeyInput = el<HTMLInputElement>("hotkey");
const applyHotkeyBtn = el<HTMLButtonElement>("apply-hotkey");
const hotkeyResult = el<HTMLSpanElement>("hotkey-result");
const pauseHotkey = el<HTMLInputElement>("pause-hotkey");
const targetLanguage = el<HTMLInputElement>("target-language");
const hideOnBlur = el<HTMLInputElement>("hide-on-blur");
const appearFocused = el<HTMLInputElement>("appear-focused");
const copyRestores = el<HTMLInputElement>("copy-restores");
const autostart = el<HTMLInputElement>("autostart");

// 翻译服务
const preset = el<HTMLSelectElement>("preset");
const baseUrl = el<HTMLInputElement>("base-url");
const apiKey = el<HTMLInputElement>("api-key");
const toggleKeyBtn = el<HTMLButtonElement>("toggle-key");
const saveKeyBtn = el<HTMLButtonElement>("save-key");
const keyHint = el<HTMLParagraphElement>("key-hint");
const model = el<HTMLInputElement>("model");
const testBtn = el<HTMLButtonElement>("test");
const testResult = el<HTMLSpanElement>("test-result");
const promptArea = el<HTMLTextAreaElement>("prompt");
const resetPromptBtn = el<HTMLButtonElement>("reset-prompt");
const promptHint = el<HTMLSpanElement>("prompt-hint");

// 外观
const background = el<HTMLInputElement>("background");
const foreground = el<HTMLInputElement>("foreground");
const fontFamily = el<HTMLInputElement>("font-family");
const fontSize = el<HTMLInputElement>("font-size");
const lineHeight = el<HTMLInputElement>("line-height");
const radius = el<HTMLInputElement>("radius");
const preview = el<HTMLDivElement>("preview");
const resetAppearanceBtn = el<HTMLButtonElement>("reset-appearance");
const appearanceHint = el<HTMLSpanElement>("appearance-hint");

// 高级
const maxChars = el<HTMLInputElement>("max-chars");
const historyEnabled = el<HTMLInputElement>("history-enabled");
const historySearch = el<HTMLInputElement>("history-search");
const historyCount = el<HTMLSpanElement>("history-count");
const clearHistoryBtn = el<HTMLButtonElement>("clear-history");
const historyList = el<HTMLDivElement>("history-list");
const loggingEnabled = el<HTMLInputElement>("logging-enabled");
const loggingLevel = el<HTMLSelectElement>("logging-level");
const logPath = el<HTMLElement>("log-path");
const exportConfigBtn = el<HTMLButtonElement>("export-config");
const importConfigBtn = el<HTMLButtonElement>("import-config");
const resetConfigBtn = el<HTMLButtonElement>("reset-config");

// 关于 / 框架
const aboutVersion = el<HTMLElement>("about-version");
const aboutIdentifier = el<HTMLElement>("about-identifier");
const aboutConfigPath = el<HTMLElement>("about-config-path");
const aboutHistoryPath = el<HTMLElement>("about-history-path");
const aboutLogPath = el<HTMLElement>("about-log-path");
const pageTitle = el<HTMLElement>("page-title");
const pageDesc = el<HTMLElement>("page-desc");
const saveHint = el<HTMLSpanElement>("save-hint");
const warningBox = el<HTMLDivElement>("config-warning");
const warningText = el<HTMLSpanElement>("config-warning-text");
const dismissWarningBtn = el<HTMLButtonElement>("dismiss-warning");
const closeBtn = el<HTMLButtonElement>("close");
const quitBtn = el<HTMLButtonElement>("quit");

const PAGE_META: Record<string, { title: string; desc: string }> = {
  general: { title: "通用", desc: "热键、目标语言与便签行为。" },
  service: { title: "翻译服务", desc: "Endpoint、API Key、模型与系统提示词。" },
  appearance: { title: "外观", desc: "主题、颜色、字体与便签圆角。" },
  advanced: { title: "高级", desc: "本地历史、日志与配置备份。" },
  about: { title: "关于", desc: "版本、数据位置与隐私说明。" },
};

const THEMES: Record<string, UiConfig> = {
  paper: {
    background: "#FFFFFF",
    foreground: "#111111",
    font_family: "SimSun",
    font_size: 16,
    line_height: 1.65,
    radius: 12,
  },
  cream: {
    background: "#FBF3D5",
    foreground: "#3B3325",
    font_family: "Microsoft YaHei",
    font_size: 16,
    line_height: 1.7,
    radius: 14,
  },
  green: {
    background: "#E8F1E4",
    foreground: "#22352A",
    font_family: "Microsoft YaHei",
    font_size: 16,
    line_height: 1.7,
    radius: 12,
  },
  night: {
    background: "#1F242B",
    foreground: "#E6E9EF",
    font_family: "Microsoft YaHei",
    font_size: 16,
    line_height: 1.7,
    radius: 12,
  },
  sepia: {
    background: "#F3E9D2",
    foreground: "#3F3526",
    font_family: "KaiTi",
    font_size: 17,
    line_height: 1.75,
    radius: 10,
  },
};

let defaultPrompt = "";
let historyEntries: HistoryEntry[] = [];
let appInfo: AppInfo | null = null;
let saveTimer: number | null = null;
let saveChain: Promise<boolean> = Promise.resolve(true);
let suppressAutosave = false;

function clampInt(input: HTMLInputElement, fallback: number, min: number, max: number): number {
  const value = Number(input.value);
  if (!Number.isFinite(value)) return fallback;
  return Math.min(max, Math.max(min, Math.round(value)));
}

function clampFloat(input: HTMLInputElement, fallback: number, min: number, max: number): number {
  const value = Number(input.value);
  if (!Number.isFinite(value)) return fallback;
  return Math.round(Math.min(max, Math.max(min, value)) * 100) / 100;
}

function setResult(target: HTMLElement, text: string, kind: "ok" | "err" | "") {
  target.textContent = text;
  target.classList.remove("ok", "err");
  if (kind) target.classList.add(kind);
}

function setSaveHint(text: string, isError = false) {
  saveHint.textContent = text;
  saveHint.classList.toggle("err", isError);
}

function applyPreview() {
  const size = clampInt(fontSize, 16, 10, 48);
  const line = clampFloat(lineHeight, 1.65, 1, 3);
  const corner = clampInt(radius, 12, 0, 32);
  preview.style.background = background.value || "#FFFFFF";
  preview.style.color = foreground.value || "#111111";
  preview.style.fontFamily = fontFamily.value.trim() || "SimSun";
  preview.style.fontSize = `${size}px`;
  preview.style.lineHeight = String(line);
  preview.style.borderRadius = `${corner}px`;
}

function currentPromptForSave(): string {
  const value = promptArea.value;
  if (value.trim() === defaultPrompt.trim()) return "";
  return value;
}

function collect(): ConfigInput {
  return {
    base_url: baseUrl.value.trim() || "https://api.deepseek.com/v1",
    model: model.value.trim() || "deepseek-chat",
    target_language: targetLanguage.value.trim() || "简体中文",
    api_key: null,
    ui: {
      background: background.value || "#FFFFFF",
      foreground: foreground.value || "#111111",
      font_family: fontFamily.value.trim() || "SimSun",
      font_size: clampInt(fontSize, 16, 10, 48),
      line_height: clampFloat(lineHeight, 1.65, 1, 3),
      radius: clampInt(radius, 12, 0, 32),
    },
    behavior: {
      hide_on_blur: hideOnBlur.checked,
      copy_restores_clipboard: copyRestores.checked,
      appear_focused: appearFocused.checked,
      pause_hotkey: pauseHotkey.checked,
      max_chars: clampInt(maxChars, 20_000, 1_000, 100_000),
    },
    prompt: { system: currentPromptForSave() },
    history: { enabled: historyEnabled.checked, limit: 200 },
    logging: { enabled: loggingEnabled.checked, level: loggingLevel.value },
  };
}

function applyServerConfig(config: ConfigView) {
  defaultPrompt = config.default_prompt;
  apiKey.placeholder = config.has_api_key ? "已保存，留空不修改" : "sk-...";
  showWarning(config.warning);
}

function queueSave(apiKeyValue: string | null = null): Promise<boolean> {
  const config = collect();
  const run = saveChain.then(async () => {
    try {
      const result = await invoke<ConfigView>("save_config", {
        config: { ...config, api_key: apiKeyValue },
      });
      applyServerConfig(result);
      const time = new Date().toLocaleTimeString("zh-CN", { hour12: false });
      setSaveHint(`已自动保存 ${time}`);
      return true;
    } catch (error) {
      setSaveHint(`保存失败：${String(error)}`, true);
      return false;
    }
  });
  saveChain = run;
  return run;
}

function scheduleSave() {
  if (suppressAutosave) return;
  if (saveTimer !== null) window.clearTimeout(saveTimer);
  saveTimer = window.setTimeout(() => {
    saveTimer = null;
    void queueSave();
  }, 400);
}

function cancelScheduledSave() {
  if (saveTimer !== null) {
    window.clearTimeout(saveTimer);
    saveTimer = null;
  }
}

function showWarning(message: string | null) {
  if (!message) {
    warningBox.classList.add("hidden");
    warningText.textContent = "";
    return;
  }
  warningText.textContent = message;
  warningBox.classList.remove("hidden");
}

function updatePromptHint() {
  const isDefault =
    promptArea.value.trim() === "" || promptArea.value.trim() === defaultPrompt.trim();
  setResult(promptHint, isDefault ? "当前使用默认提示词" : "当前使用自定义提示词", "");
}

function inferPreset(): string {
  const base = baseUrl.value.toLowerCase();
  return base.includes("api.deepseek.com") && model.value.trim() === "deepseek-chat"
    ? "deepseek"
    : "custom";
}

function fillConfig(config: ConfigView, status: HotkeyStatus) {
  suppressAutosave = true;
  try {
    baseUrl.value = config.base_url || "https://api.deepseek.com/v1";
    model.value = config.model || "deepseek-chat";
    targetLanguage.value = config.target_language || "简体中文";
    preset.value = inferPreset();
    apiKey.value = "";
    apiKey.placeholder = config.has_api_key ? "已保存，留空不修改" : "sk-...";
    keyHint.textContent = config.has_api_key
      ? "API Key 已保存在系统凭据库；留空不会修改。"
      : "还没有保存 API Key。填写后点「保存 Key」。";

    defaultPrompt = config.default_prompt;
    promptArea.value = config.prompt || config.default_prompt;
    updatePromptHint();

    background.value = config.ui.background || "#FFFFFF";
    foreground.value = config.ui.foreground || "#111111";
    fontFamily.value = config.ui.font_family || "SimSun";
    fontSize.value = String(config.ui.font_size || 16);
    lineHeight.value = String(config.ui.line_height || 1.65);
    radius.value = String(config.ui.radius ?? 12);
    applyPreview();

    hideOnBlur.checked = config.behavior.hide_on_blur;
    copyRestores.checked = config.behavior.copy_restores_clipboard;
    appearFocused.checked = config.behavior.appear_focused;
    pauseHotkey.checked = status.paused;
    maxChars.value = String(config.behavior.max_chars || 20_000);

    historyEnabled.checked = config.history.enabled;
    loggingEnabled.checked = config.logging.enabled;
    loggingLevel.value = config.logging.level || "error";

    hotkeyInput.value = status.hotkey || config.hotkey || "Alt+Q";
    applyHotkeyStatus(status);
    showWarning(config.warning);
  } finally {
    suppressAutosave = false;
  }
}

function applyHotkeyStatus(status: HotkeyStatus) {
  hotkeyInput.value = status.hotkey;
  pauseHotkey.checked = status.paused;
  if (status.error) {
    setResult(hotkeyResult, status.error, "err");
  } else if (status.paused) {
    setResult(hotkeyResult, "热键已暂停", "");
  } else {
    setResult(hotkeyResult, "当前热键已生效", "ok");
  }
}

function applyTheme(name: string) {
  const theme = THEMES[name];
  if (!theme) return;
  background.value = theme.background;
  foreground.value = theme.foreground;
  fontFamily.value = theme.font_family;
  fontSize.value = String(theme.font_size);
  lineHeight.value = String(theme.line_height);
  radius.value = String(theme.radius);
  applyPreview();
  markActiveTheme(name);
}

function markActiveTheme(name: string) {
  document.querySelectorAll<HTMLButtonElement>(".theme-btn").forEach((button) => {
    button.classList.toggle("active", button.dataset.theme === name);
  });
}

async function refreshHistory() {
  try {
    historyEntries = await invoke<HistoryEntry[]>("get_history");
    renderHistory();
  } catch (error) {
    setSaveHint(`读取历史失败：${String(error)}`, true);
  }
}

function truncate(text: string, limit: number): string {
  return text.length > limit ? `${text.slice(0, limit)}…` : text;
}

function formatTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString("zh-CN", { hour12: false });
}

function renderHistory() {
  const query = historySearch.value.trim().toLowerCase();
  historyCount.textContent = `${historyEntries.length} / 200`;
  historyList.replaceChildren();

  if (historyEntries.length === 0) {
    const empty = document.createElement("p");
    empty.className = "history-empty";
    empty.textContent = historyEnabled.checked
      ? "还没有历史记录。完成一次翻译后会出现在这里。"
      : "历史已关闭，不会记录新的翻译；已有记录仍可查看或清空。";
    historyList.append(empty);
    return;
  }

  const filtered = historyEntries.filter((entry) => {
    if (!query) return true;
    return (
      entry.source.toLowerCase().includes(query) ||
      entry.translation.toLowerCase().includes(query)
    );
  });

  if (filtered.length === 0) {
    const empty = document.createElement("p");
    empty.className = "history-empty";
    empty.textContent = "没有匹配的历史记录。";
    historyList.append(empty);
    return;
  }

  for (const entry of filtered) {
    const item = document.createElement("article");
    item.className = "history-item";

    const head = document.createElement("div");
    head.className = "history-head";
    const meta = document.createElement("span");
    meta.className = "history-meta";
    meta.textContent = `${formatTime(entry.time)} · ${entry.target_language} · ${entry.model} · ${(entry.duration_ms / 1000).toFixed(1)}s`;
    const actions = document.createElement("div");
    actions.className = "history-actions";

    const copyButton = document.createElement("button");
    copyButton.type = "button";
    copyButton.className = "ghost-btn";
    copyButton.textContent = "复制译文";
    copyButton.addEventListener("click", async () => {
      try {
        await writeText(entry.translation);
        setSaveHint("译文已复制");
      } catch (error) {
        setSaveHint(`复制失败：${String(error)}`, true);
      }
    });

    const deleteButton = document.createElement("button");
    deleteButton.type = "button";
    deleteButton.className = "ghost-btn danger-text";
    deleteButton.textContent = "删除";
    deleteButton.addEventListener("click", async () => {
      try {
        historyEntries = await invoke<HistoryEntry[]>("delete_history", { id: entry.id });
        renderHistory();
        setSaveHint("记录已删除");
      } catch (error) {
        setSaveHint(`删除失败：${String(error)}`, true);
      }
    });

    actions.append(copyButton, deleteButton);
    head.append(meta, actions);

    const source = document.createElement("p");
    source.className = "history-source";
    source.textContent = truncate(entry.source.replace(/\s+/g, " "), 160);

    const translation = document.createElement("p");
    translation.className = "history-translation";
    translation.textContent = truncate(entry.translation.replace(/\s+/g, " "), 240);

    item.append(head, source, translation);
    historyList.append(item);
  }
}

async function confirmDialog(message: string): Promise<boolean> {
  try {
    return await ask(message, { title: "LingoNote", kind: "warning" });
  } catch {
    return window.confirm(message);
  }
}

function fillAppInfo(info: AppInfo) {
  appInfo = info;
  aboutVersion.textContent = info.version;
  aboutIdentifier.textContent = info.identifier;
  aboutConfigPath.textContent = info.config_path;
  aboutHistoryPath.textContent = info.history_path;
  aboutLogPath.textContent = info.log_dir;
  logPath.textContent = info.log_dir;
}

async function saveApiKey() {
  const value = apiKey.value.trim();
  if (!value) {
    keyHint.textContent = "请输入 API Key 后再保存。";
    return;
  }
  saveKeyBtn.disabled = true;
  try {
    const ok = await queueSave(value);
    if (ok) {
      apiKey.value = "";
      keyHint.textContent = "API Key 已保存到系统凭据库，之后留空即可。";
    }
  } finally {
    saveKeyBtn.disabled = false;
  }
}

async function load() {
  const [config, status, info] = await Promise.all([
    invoke<ConfigView>("get_config"),
    invoke<HotkeyStatus>("get_hotkey_status"),
    invoke<AppInfo>("get_app_info"),
  ]);
  fillConfig(config, status);
  fillAppInfo(info);

  try {
    autostart.checked = await invoke<boolean>("get_autostart");
  } catch (error) {
    autostart.checked = false;
    autostart.disabled = true;
    setSaveHint(`无法读取开机自启状态：${String(error)}`, true);
  }

  await refreshHistory();
}

function showPage(name: string) {
  document.querySelectorAll<HTMLButtonElement>(".nav-btn").forEach((button) => {
    button.classList.toggle("active", button.dataset.page === name);
  });
  document.querySelectorAll<HTMLDivElement>(".page").forEach((page) => {
    page.classList.toggle("active", page.dataset.page === name);
  });
  const meta = PAGE_META[name];
  if (meta) {
    pageTitle.textContent = meta.title;
    pageDesc.textContent = meta.desc;
  }
}

// ---------- 事件绑定 ----------

document.querySelectorAll<HTMLButtonElement>(".nav-btn").forEach((button) => {
  button.addEventListener("click", () => showPage(button.dataset.page ?? "general"));
});

document.querySelectorAll<HTMLButtonElement>(".theme-btn").forEach((button) => {
  button.addEventListener("click", () => {
    applyTheme(button.dataset.theme ?? "paper");
    scheduleSave();
  });
});

for (const input of [background, foreground, fontFamily, fontSize, lineHeight, radius]) {
  input.addEventListener("input", () => {
    applyPreview();
    scheduleSave();
  });
}

for (const input of [baseUrl, model, targetLanguage, maxChars, promptArea]) {
  input.addEventListener("input", () => {
    if (input === promptArea) updatePromptHint();
    scheduleSave();
  });
}

for (const input of [hideOnBlur, appearFocused, copyRestores, loggingEnabled]) {
  input.addEventListener("change", () => scheduleSave());
}

preset.addEventListener("change", () => {
  if (preset.value === "deepseek") {
    baseUrl.value = "https://api.deepseek.com/v1";
    model.value = "deepseek-chat";
  }
  scheduleSave();
});

for (const input of [baseUrl, model]) {
  input.addEventListener("change", () => {
    preset.value = inferPreset();
  });
}

loggingLevel.addEventListener("change", () => scheduleSave());

historyEnabled.addEventListener("change", () => {
  scheduleSave();
  void refreshHistory();
});

historySearch.addEventListener("input", () => renderHistory());

resetPromptBtn.addEventListener("click", () => {
  promptArea.value = defaultPrompt;
  updatePromptHint();
  scheduleSave();
});

resetAppearanceBtn.addEventListener("click", () => {
  applyTheme("paper");
  setResult(appearanceHint, "已恢复默认外观", "ok");
  scheduleSave();
});

applyHotkeyBtn.addEventListener("click", async () => {
  applyHotkeyBtn.disabled = true;
  setResult(hotkeyResult, "应用中…", "");
  try {
    const status = await invoke<HotkeyStatus>("set_hotkey", {
      hotkey: hotkeyInput.value.trim(),
    });
    applyHotkeyStatus(status);
    setSaveHint(status.paused ? "已保存热键；当前处于暂停状态" : "热键已生效");
  } catch (error) {
    setResult(hotkeyResult, String(error), "err");
  } finally {
    applyHotkeyBtn.disabled = false;
  }
});

pauseHotkey.addEventListener("change", async () => {
  const want = pauseHotkey.checked;
  pauseHotkey.disabled = true;
  try {
    const status = await invoke<HotkeyStatus>("set_hotkey_paused", { paused: want });
    applyHotkeyStatus(status);
    setSaveHint(want ? "已暂停全局热键" : "全局热键已恢复");
  } catch (error) {
    pauseHotkey.checked = !want;
    setResult(hotkeyResult, String(error), "err");
  } finally {
    pauseHotkey.disabled = false;
  }
});

autostart.addEventListener("change", async () => {
  const want = autostart.checked;
  autostart.disabled = true;
  try {
    const enabled = await invoke<boolean>("set_autostart", { enabled: want });
    autostart.checked = enabled;
    setSaveHint(enabled ? "已开启开机自启" : "已关闭开机自启");
  } catch (error) {
    autostart.checked = !want;
    setSaveHint(`设置开机自启失败：${String(error)}`, true);
  } finally {
    autostart.disabled = false;
  }
});

toggleKeyBtn.addEventListener("click", () => {
  const hidden = apiKey.type === "password";
  apiKey.type = hidden ? "text" : "password";
  toggleKeyBtn.textContent = hidden ? "隐藏" : "显示";
});

saveKeyBtn.addEventListener("click", () => void saveApiKey());
apiKey.addEventListener("change", () => {
  if (apiKey.value.trim()) void saveApiKey();
});

testBtn.addEventListener("click", async () => {
  testBtn.disabled = true;
  setResult(testResult, "测试中…", "");
  try {
    const result = await invoke<string>("test_connection", {
      config: { ...collect(), api_key: apiKey.value.trim() || null },
    });
    setResult(testResult, result, "ok");
  } catch (error) {
    setResult(testResult, String(error), "err");
  } finally {
    testBtn.disabled = false;
  }
});

exportConfigBtn.addEventListener("click", async () => {
  try {
    const path = await save({
      title: "导出 LingoNote 配置",
      defaultPath: "lingonote-config.json",
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    if (!path) return;
    const written = await invoke<string>("export_config", { path });
    const name = written.split(/[\\/]/).pop() ?? written;
    setSaveHint(`已导出：${name}`);
  } catch (error) {
    setSaveHint(`导出失败：${String(error)}`, true);
  }
});

importConfigBtn.addEventListener("click", async () => {
  try {
    const selected = await open({
      title: "导入 LingoNote 配置",
      multiple: false,
      filters: [{ name: "JSON", extensions: ["json"] }],
    });
    const path = Array.isArray(selected) ? selected[0] : selected;
    if (!path) return;
    if (
      !(await confirmDialog(
        "导入会覆盖当前配置（API Key 保留本机已有值），并先把旧配置备份为 config.backup.json。继续？",
      ))
    ) {
      return;
    }
    cancelScheduledSave();
    await saveChain;
    const config = await invoke<ConfigView>("import_config", { path });
    const status = await invoke<HotkeyStatus>("get_hotkey_status");
    fillConfig(config, status);
    await refreshHistory();
    setSaveHint("配置已导入");
  } catch (error) {
    setSaveHint(`导入失败：${String(error)}`, true);
  }
});

resetConfigBtn.addEventListener("click", async () => {
  if (
    !(await confirmDialog(
      "恢复全部默认设置？API Key 会保留，当前配置会先备份为 config.backup.json。",
    ))
  ) {
    return;
  }
  try {
    cancelScheduledSave();
    await saveChain;
    const config = await invoke<ConfigView>("reset_config");
    const status = await invoke<HotkeyStatus>("get_hotkey_status");
    fillConfig(config, status);
    await refreshHistory();
    setSaveHint("已恢复默认设置");
  } catch (error) {
    setSaveHint(`恢复默认失败：${String(error)}`, true);
  }
});

clearHistoryBtn.addEventListener("click", async () => {
  if (historyEntries.length === 0) return;
  if (!(await confirmDialog("确定清空全部本地历史？此操作不可撤销。"))) return;
  try {
    historyEntries = await invoke<HistoryEntry[]>("clear_history");
    renderHistory();
    setSaveHint("历史已清空");
  } catch (error) {
    setSaveHint(`清空失败：${String(error)}`, true);
  }
});

dismissWarningBtn.addEventListener("click", () => {
  showWarning(null);
  void invoke("dismiss_config_warning");
});

closeBtn.addEventListener("click", () => void invoke("close_settings"));
quitBtn.addEventListener("click", () => void invoke("quit_app"));

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    void invoke("close_settings");
  }
});

window.addEventListener("beforeunload", () => {
  if (saveTimer !== null) {
    cancelScheduledSave();
    void queueSave();
  }
});

void listen<HotkeyStatus>("hotkey-changed", (event) => applyHotkeyStatus(event.payload));
void listen<boolean>("autostart-changed", (event) => {
  autostart.checked = event.payload;
});

void load().catch((error) => {
  setSaveHint(`读取配置失败：${String(error)}`, true);
});
