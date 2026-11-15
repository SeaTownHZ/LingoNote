import { invoke } from "@tauri-apps/api/core";

/** get_config 只用于读便签尺寸做轮廓预览，其余字段忽略。 */
interface ConfigViewLike {
  window?: {
    note_width?: number;
    note_height?: number;
  };
}

const DEFAULT_NOTE_WIDTH = 420;
const DEFAULT_NOTE_HEIGHT = 360;
const MIN_NOTE_WIDTH = 320;
const MAX_NOTE_WIDTH = 900;
const MIN_NOTE_HEIGHT = 220;
const MAX_NOTE_HEIGHT = 900;

const el = <T extends HTMLElement = HTMLElement>(id: string): T => {
  const node = document.getElementById(id);
  if (!node) throw new Error(`missing element #${id}`);
  return node as T;
};

const tip = el<HTMLDivElement>("tip");
const outline = el<HTMLDivElement>("outline");

/** 遮罩内的确认/取消只允许发一次，避免双击重复 invoke。 */
let busy = false;

function clampSize(
  value: number | undefined,
  fallback: number,
  min: number,
  max: number,
): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return fallback;
  return Math.min(max, Math.max(min, Math.round(value)));
}

/** 轮廓左上角跟随鼠标（CSS 像素，仅作预览；真实坐标由 Rust 用 GetCursorPos 读取）。 */
function moveOutline(event: MouseEvent) {
  outline.style.left = `${event.clientX}px`;
  outline.style.top = `${event.clientY}px`;
  outline.style.display = "block";
}

async function loadNoteSize() {
  let width = DEFAULT_NOTE_WIDTH;
  let height = DEFAULT_NOTE_HEIGHT;
  try {
    const config = await invoke<ConfigViewLike>("get_config");
    width = clampSize(config.window?.note_width, DEFAULT_NOTE_WIDTH, MIN_NOTE_WIDTH, MAX_NOTE_WIDTH);
    height = clampSize(
      config.window?.note_height,
      DEFAULT_NOTE_HEIGHT,
      MIN_NOTE_HEIGHT,
      MAX_NOTE_HEIGHT,
    );
  } catch {
    // 读不到配置就按默认 420×360 画轮廓，遮罩仍可正常取点。
  }
  outline.style.width = `${width}px`;
  outline.style.height = `${height}px`;
}

function cancelPick() {
  if (busy) return;
  busy = true;
  void invoke("cancel_position_pick").catch(() => {
    // 取消失败也不能把遮罩卡死，允许用户再按一次 Esc。
    busy = false;
  });
}

function confirmPick() {
  if (busy) return;
  busy = true;
  void invoke("confirm_position_pick").catch((error) => {
    busy = false;
    tip.textContent = `取点失败：${String(error)} · Esc 取消`;
  });
}

window.addEventListener("mousemove", moveOutline);

window.addEventListener("click", (event) => {
  event.preventDefault();
  confirmPick();
});

window.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  event.preventDefault();
  cancelPick();
});

window.addEventListener("contextmenu", (event) => {
  event.preventDefault();
  cancelPick();
});

// 防止拖拽把遮罩页当成图片/文本拖走。
document.addEventListener("dragstart", (event) => event.preventDefault());

outline.style.width = `${DEFAULT_NOTE_WIDTH}px`;
outline.style.height = `${DEFAULT_NOTE_HEIGHT}px`;
void loadNoteSize();
