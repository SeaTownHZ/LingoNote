# SPEC · M2.5 便签位置与不透明度

- 项目：LingoNote（语签）v0.2.0 → 0.2.5 试点里程碑
- 来源：`docs/需求文档-v0.3.md` FR-06（出现位置）/ FR-07（不透明度）中尚未实现的 P1 项
- 负责人：Router/PM（本会话 lead）
- 状态：spec_ready（等待用户确认后执行）
- 关联：`project/PLAN.md`、`project/TASKS.json`、`project/STATUS.md`、`project/DECISIONS.md`

## 1. 目标

让便签的"出现在哪、有多透明"从固定行为变成可配置、可记忆：

1. **位置三模式**（互斥，默认跟随鼠标）：
   - `cursor` 跟随鼠标：出现在光标右下 12px，空间不足自动翻转，不越出工作区（保持现行行为）。
   - `anchor` 固定位置：屏幕九宫格锚点 + 边距（横/纵各 0–400px），相对**光标所在显示器**的工作区。
   - `remember` 记住上次：用户拖动便签后保存坐标；下次出现在同一位置；坐标越界时夹回工作区。
2. **不透明度**：80%–100%（整数百分比，默认 100%），实时生效、重启仍生效。
3. **设置页**：外观分区加不透明度滑杆；通用分区加出现位置模式、锚点、边距；修改自动保存，无需点保存。

## 2. 非目标

- 不做窗口宽高自定义、边缘拖拽缩放（M3 再说）。
- 不做"鼠标离开自动隐藏"、固定锚点的多屏记忆策略（记住的是绝对坐标，不是"某块屏的右下角"）。
- 不改系统提示词、翻译链路、历史、日志、托盘。
- 不新增 IPC 命令；不改 `SCHEMA_VERSION`（保持 2，纯增量字段）。

## 3. 配置契约（Rust ↔ TS 必须逐字一致）

`config.json` 增量（其余字段不变）：

```json
{
  "schema_version": 2,
  "ui": {
    "background": "#FFFFFF",
    "foreground": "#111111",
    "font_family": "SimSun",
    "font_size": 16,
    "line_height": 1.65,
    "radius": 12,
    "opacity": 100
  },
  "window": {
    "position_mode": "cursor",
    "anchor": "bottom-right",
    "margin_x": 16,
    "margin_y": 16,
    "last_x": null,
    "last_y": null
  }
}
```

规则：

| 字段 | 类型 | 默认 | 合法范围 / 枚举 | 归一化 |
|---|---|---|---|---|
| `ui.opacity` | u32 | 100 | 80–100 | 超界钳制 |
| `window.position_mode` | string | `cursor` | `cursor` / `anchor` / `remember` | 未知值回落 `cursor` |
| `window.anchor` | string | `bottom-right` | `top-left` `top-center` `top-right` `center-left` `center` `center-right` `bottom-left` `bottom-center` `bottom-right` | 未知值回落 `bottom-right` |
| `window.margin_x` | i32 | 16 | 0–400 | 钳制 |
| `window.margin_y` | i32 | 16 | 0–400 | 钳制 |
| `window.last_x` / `last_y` | Option<i32> | null | 任意 i32 | 使用时夹回工作区；设置页不发送（省略即 null） |

`ConfigView` 与 `ConfigInput` 都必须带上 `window`（`ConfigInput.window` 用 `#[serde(default)]`，老版本设置页不发该字段也能工作）。`save_config` 必须**保留**已有 `window.last_x/last_y`：入参为 `None` 时取磁盘上的旧值，不能被设置页的自动保存清掉。

## 4. 行为规格

### 4.1 位置计算（Windows；非 Windows 保持现状）

抽成 `pure.rs` 纯函数（便于测试）：

```
cursor_position(cursor, win, work, gap) -> (x, y)
anchor_position(anchor, win, work, margin) -> (x, y)
clamp_position(pos, win, work) -> (x, y)
```

- `work` 为显示器工作区（`rcWork`，排除任务栏）。
- `cursor`：`x = cursor.x + 12`；若 `x + win_w > work.right` 则翻到 `x = cursor.x - win_w - 12`；y 同理；最后夹回工作区。
- `anchor`：九宫格，边距为到工作区边缘的距离；`center` 居中。
- `remember`：`last_x/last_y` 都有 → 直接夹回工作区；否则回落 `cursor`。
- 显示器选择：`cursor` / `anchor` 用光标点最近的显示器（现状）；`remember` 用保存点最近的显示器，点为 None 时回退光标。

### 4.2 记住位置（Rust 侧）

- `note` 窗口 `WindowEvent::Moved` → 仅当 `window.position_mode == "remember"` 时记录。
- 去抖：约 600ms 内无新移动再落盘（原子写沿用现有 `write_stored_config`），避免拖动时每帧写盘。
- 落盘内容只改 `window.last_x/last_y`，其它字段以磁盘当前值为准；不 emit `config-changed`（避免便签刷新循环）。
- 拖动位置在程序化 `show_note` 定位时也会触发 Moved：写入值与当前展示位置一致，属可接受冗余。

### 4.3 不透明度（前端）

- `main.ts` 的 `applyUi` 增加：`--note-opacity: opacity/100`（钳制 80–100）。
- `styles.css`：`.note-shell { opacity: var(--note-opacity, 1); }`，保证整张便签（含文字）一起变透明，且不影响现有阴影/圆角。
- 设置页预览同步使用该变量；主题预设与"恢复默认外观"把不透明度置 100。

### 4.4 设置页交互

- 通用分区：
  - 「出现位置」`<select id="position-mode">`：跟随鼠标 / 固定位置 / 记住上次位置。
  - 「锚点」`<select id="anchor">` 九宫格中文标签；「横向边距」「纵向边距」`<input type="number" id="margin-x|margin-y" min=0 max=400>`。
  - 非 `anchor` 模式下锚点/边距置灰（disabled），`remember` 模式显示提示"拖动便签后会自动记住位置"。
- 外观分区：`<input type="range" id="opacity" min="80" max="100" step="1">` + `<span id="opacity-value">`，实时预览。
- 所有修改走现有 `scheduleSave()` 自动保存；`applyTheme` / 恢复默认外观同步 opacity=100。
- 新增元素必须加进 `settings.ts` 的 `el(...)` 绑定，缺失即启动报错（宁可显式失败）。

## 5. 验收标准（Given / When / Then）

| 编号 | 前提 | 操作 | 预期 |
|---|---|---|---|
| AC-M25-1 | 默认配置 | 首次启动 / 读 `config.json` | `ui.opacity=100`、`window` 默认值齐全；旧 v2 配置无 `window` 字段也能启动 |
| AC-M25-2 | 设置里把不透明度调 80 | 触发一次翻译 | 便签整体 80% 透明；重启后仍 80% |
| AC-M25-3 | 位置模式=anchor，锚点=bottom-right，边距 24/24 | 在任意屏按热键 | 便签右下角距工作区右/下边各 24px |
| AC-M25-4 | 位置模式=anchor | 分别在 9 个锚点各显示一次 | 位置符合九宫格定义；越界被夹回工作区 |
| AC-M25-5 | 位置模式=remember，拖动便签到 A 点 | 隐藏再显示（或重启后显示） | 便签出现在 A 点附近（±2px） |
| AC-M25-6 | remember 模式，`last_x/y` 指向屏幕外 | 显示便签 | 被夹回保存点最近显示器的工作区内（无保存点回退光标屏），不消失在屏外 |
| AC-M25-7 | 现有配置 | `cargo test` | 原有 16 个测试全过 + 新增纯函数测试全过（≥20 个） |
| AC-M25-8 | 干净依赖 | `npm run build` | tsc 类型检查 + vite 构建通过，无 TS 报错 |
| AC-M25-9 | 手动/烟测 | 设置页五个分区切换、外观预览、自动保存 | 无 JS 报错；`config.json` 对应字段即时更新 |
| AC-M25-10 | 回归 | 热键/翻译/复制/历史/导入导出 | 行为不变 |

## 6. Definition of Done（硬闸，缺一不可）

1. `npm run build` 通过（tsc + vite）。
2. `cd src-tauri && cargo test --release --offline` 全绿，且测试数 ≥ 20。
3. 生产构建 `npm run tauri build -- --no-bundle` 通过（**不要用裸 `cargo build --release`**：Tauri 会保留 devUrl，产物白屏；见 DECISIONS D9）。
4. 每条任务提交"证据块"：命令 + 关键输出 + 产物路径，写入 `project/STATUS.md`。
5. 独立评审通过：评审者只读 diff 与 SPEC，P0/P1 问题清零（T3 报告 `project/REVIEW.md`）。
6. 验收者按 AC-M25-1..10 逐条核验，写 `project/ACCEPT.md`；不通过则打回实现者，不做"口头通过"。
7. `git commit` 一个里程碑提交，message 带 `M2.5`。

## 7. 写入域（并行开发的边界，越界即冲突）

| 任务 | 允许改的文件 |
|---|---|
| T1 Rust | `src-tauri/src/pure.rs`、`src-tauri/src/lib.rs`、`src-tauri/tests/pure.rs` |
| T2 前端 | `src/main.ts`、`src/settings.ts`、`src/styles.css`、`settings.html` |
| T3 评审 | `project/REVIEW.md` |
| T4 验收 | `project/ACCEPT.md`、`project/STATUS.md` |

任何一方需要动别人的文件，先在 `project/STATUS.md` 记一条阻塞，交回 PM 协调。
