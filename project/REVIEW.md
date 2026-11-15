# REVIEW · M2.5 便签位置与不透明度（T3 独立评审 · 重试）

- 评审人：reviewer2（独立于 T1/T2 实现者）
- 评审对象：`D:\ClipTonge` 工作区未提交改动（7 个文件，831+/33-）+ `project/SPEC.md` §3–§7
- 评审方式：只读。`git diff --stat` / `git diff -U3` 逐块核对 + 按行号精准读取上下文；**未修改任何实现文件、未 git commit、未复跑构建**（构建与测试证据引用 `project/STATUS.md`）
- 评审时间：2026-11-15

## 结论

**PASS** — P0 = 0，P1 = 0，P2 = 2。**无阻断项**，可进入 T4 验收。

要点：

- 契约字段名/类型/默认值/归一化与 SPEC §3 逐字一致（`src-tauri/src/pure.rs:16-21,45-56,72-98,301-327,378-380`）。
- `ConfigView` / `ConfigInput` 均带 `window`，且 `ConfigInput.window` 有 `#[serde(default)]`（`src-tauri/src/pure.rs:187,223,242-243`）；`save_config` 返回值经 `build_config_view` 带 `window`（`src-tauri/src/lib.rs:226`）。
- `save_config` 保留磁盘上的 `last_x/last_y`（`src-tauri/src/lib.rs:263-276`）；Moved 去抖 600ms、仅在 `remember` 模式落盘、只改 `last_*` 且不 emit（`src-tauri/src/lib.rs:846-903,1457`）。
- opacity 只在 `.note-shell` 生效一次，未与 `.note-content` 或设置页预览双重叠加（`src/styles.css:30` 与 `:515` 分属 `index.html:10` 和 `settings.html:255` 两个窗口；`src/styles.css` 中 `.note-content` 无 opacity）。
- 设置页 7 个新元素全部 `el(...)` 绑定（`src/settings.ts:128-132,156-157`）；payload 含 `ui.opacity` 与 `window`（`src/settings.ts:366-383`）；主题预设与“恢复默认外观”把不透明度复位 100（`src/settings.ts:530,792-796`）。
- 工作区 7 个改动文件全部落在 SPEC §7 的 T1/T2 写入域内，无越界文件；`project/` 为未跟踪目录，本次只写 `project/REVIEW.md`。

## 问题表

| severity | 文件:行 | 问题 | 影响 | 建议 |
|---|---|---|---|---|
| P2 | `src-tauri/src/lib.rs:714-730`（另见 `:757-762`）；对照 `project/SPEC.md:83` 与 `project/SPEC.md:117` | remember 模式的显示器选择用「保存点最近的显示器」（与 SPEC §4.1 一致），但 AC-M25-6 文字要求「夹回**光标所在屏**工作区内」。两处口径在“保存点离光标屏很远且原显示器已离线”时结论不同 | 实现本身符合更具体的 §4.1；但 T4 若按 AC-M25-6 字面验收，可能误判 FAIL（或用户期望便签出现在当前操作的屏） | 不改代码。建议 PM 在 ACCEPT 中明确“以 §4.1 保存点最近屏为准”，或补一条 SPEC 澄清；若要改行为则需 PM 决策，不在评审内放宽 |
| P2 | `src-tauri/src/lib.rs:887-903`（position 落盘：读盘→改 `last_*`→写盘）与 `src-tauri/src/lib.rs:245-292`（`save_config`：同样读盘→写整份配置） | 两处 read-modify-write 之间无互斥/版本校验。仅在“拖动便签结束后 600ms 去抖窗口内，设置页恰好完成一次自动保存”时，后写者会用旧快照覆盖对方刚写的字段（丢一次 `last_*`，或反向覆盖 `position_mode/anchor/margin`） | 概率极低（需用户“拖完立刻改设置”且两者写盘交错），最坏丢一次位置/设置更新，不损坏文件（原子写） | M2.5 可接受，记为已知限制；后续可把窗口坐标落盘并入同一写锁/单写线程，或写前比较 `mtime` |

> 未发现 P0/P1：无契约字段错名/漏字段、无未归一化输入、无 last_* 被设置页清空、无越界写入、无明显回归。

## 核对表（6 组）

| # | 核对项 | 结论 | 证据 |
|---|---|---|---|
| 1 | 改动范围与写入域：只改 7 个实现文件、无越界 | **通过** | `git diff --stat` = `settings.html / src-tauri/src/lib.rs / src-tauri/src/pure.rs / src-tauri/tests/pure.rs / src/main.ts / src/settings.ts / src/styles.css`，合计 831+/33-；`git status --porcelain` 仅这 7 个 `M` + 未跟踪 `project/`。对照 SPEC §7（`project/SPEC.md:133-141`）：T1 三文件、T2 四文件，无交叉。 |
| 2 | 配置契约（SPEC §3）：字段名/类型/默认值/范围/归一化；`ConfigView`/`ConfigInput` 带 `window`；旧 v2 配置无 `window` 可启动 | **通过** | `ui.opacity: u32` 默认 100、80–100 钳制（`pure.rs:16-19,54-55,66,378`）；`WindowConfig` 六字段类型/默认与 SPEC 逐字一致（`pure.rs:72-98`）；未知 mode→cursor、未知 anchor→bottom-right、margin 0–400（`pure.rs:301-327,378-380`）；`ConfigView.window`（`pure.rs:223`）、`ConfigInput.window` + `#[serde(default)]`（`pure.rs:242-243`）；`StoredConfig` 容器级 `#[serde(default)]`（`pure.rs:177-178`）保证旧配置缺 `window` 用默认值，测试覆盖（`tests/pure.rs:303-323`）；`schema_version` 仍为 2（`pure.rs:9`）。 |
| 3 | 位置纯函数（SPEC §4.1）：cursor 12px 偏移/翻转、九宫格 9 锚点、margin、越界夹回、center 居中、remember 夹回；lib.rs 按模式分派 | **通过** | `cursor_position` 翻转+末端 clamp（`pure.rs:448-465`）；`anchor_position` 九宫格/margin/center/clamp（`pure.rs:498-520`）；`clamp_position` 含“窗口比工作区大”退化（`pure.rs:437-446`）；`show_note` 按 mode 三分支分派（`lib.rs:750-764`），remember 且 `last_*` 齐全时 `clamp_position`，否则回落 cursor（`:757-762`），与非 Windows 保持现状一致（cfg 仅 Windows）。测试：翻转 `tests/pure.rs:341-366`、越界夹回 `:369-387`、九宫格 9 锚点 `:389-412`、margin/超大窗口 `:415-435`、未知 anchor `:326-338`。 |
| 4 | 记住位置（SPEC §4.2）：`save_config` 保留 `last_*`；仅 remember 记录；600ms 去抖；只改 `last_*`、另一份配置以磁盘为准、不 emit；线程无死循环/不丢最后位置 | **通过** | 保留逻辑 `lib.rs:263-276`（None 才取磁盘旧值 → 设置页显式发 null 不会清空），`normalize_config` 不裁剪 `last_*`（`pure.rs:320-327`，测试 `tests/pure.rs:268-300`）；记录链 `Moved`→`note_remembers_position()`→`schedule_note_position_save`（`lib.rs:906-922`），落盘前二次校验 mode（`:889-892`）；去抖 `Duration 600ms` + 单线程哨兵 `NOTE_MOVE_DEBOUNCING`（`lib.rs:820-884`），检查了 store(false)/take 的竞态：无论新事件落在哪个窗口都会被“旧线程 continue”或“新线程接管”之一处理，**无死循环、不丢最后一次位置**；`persist_note_position` 只改 `window.last_x/last_y` 且不 `emit`（`lib.rs:887-903`）；mode 缓存在 setup/save/import/reset/show_note 五处刷新（`lib.rs:287,352,383,670,907`）。 |
| 5 | 前端不透明度（SPEC §4.3）：`applyUi` 设 `--note-opacity = opacity/100` 并按 80–100 钳制；仅在 `.note-shell` 生效一次，不双重叠加、不破坏阴影/圆角；设置页预览同步、主题/恢复默认复位 100 | **通过** | `normalizeOpacity` 缺失回 100、非法值回 100、80–100 取整（`src/main.ts:83-86,96`）；`.note-shell { opacity: var(--note-opacity, 1) }`（`src/styles.css:30`），`.note-content` 无 opacity（全文仅 2 处 `--note-opacity`：`styles.css:30` 在 `index.html:10` 的便签卡上，`styles.css:515` 在 `settings.html:255` 的预览块上，**两个窗口互不嵌套，不是叠加**）；`.preview-note` 用同一变量（`styles.css:504-516`）；`applyPreview` 同步写变量与 `#opacity-value`（`src/settings.ts:332-344`）；`applyTheme` 与 `reset-appearance`（paper）都复位 opacity=100（`src/settings.ts:520-533,792-796`）。 |
| 6 | 设置页（SPEC §4.4）：7 个新 id、`el()` 绑定、非 anchor 模式禁用锚点/边距、payload 含 `ui.opacity` 与 `window`、旧配置无 `window` 兜底、自动保存挂接 | **通过** | 7 个 id 唯一存在：`settings.html:105,113,128,132,135,250,251`（position-mode/anchor/margin-x/margin-y/position-hint/opacity/opacity-value），绑定 `src/settings.ts:128-132,156-157`，`el()` 缺元素直接抛错（`:113-117`）；`updatePositionUi` 非 anchor 置 disabled、remember/cursor 提示文案（`:306-320`），change 时先更新 UI 再 scheduleSave（`:741-745`）；`collect()` 发送 `ui.opacity` 与完整 `window`（`:366-383`）；`fillConfig` 对 `config.window` 用 `?? DEFAULT_WINDOW_CONFIG` 兜底、对 opacity/margin 归一化（`:478-485`）；opacity/anchor/margin 监听均走 `scheduleSave()`（`:736-750`）。 |

## AC 预判（静态可判部分，供 T4 参考）

| AC | 预判 | 依据 |
|---|---|---|
| AC-M25-1 | 通过（静态） | 默认 opacity=100、window 默认齐全（`pure.rs:66,91-98`）；旧 v2 无 window 用容器级 default（`pure.rs:177-178`，测试 `tests/pure.rs:303-323`） |
| AC-M25-2 | 代码路径通过，视觉需真机 | 保存 `ui.opacity`（`settings.ts:366`）→ 启动 `applyUi`（`main.ts:96`）→ 整卡 `opacity`（`styles.css:30`）；“整体 80%”需肉眼/截图确认 |
| AC-M25-3 | 通过（静态） | bottom-right 24/24 → `work.right-win_w-24`、`work.bottom-win_h-24`（`pure.rs:498-517`，margin 测试 `tests/pure.rs:415-424`） |
| AC-M25-4 | 通过（静态） | 9 锚点逐点断言（`tests/pure.rs:389-412`）+ 越界 clamp（`:426-434`） |
| AC-M25-5 | 需运行时 | 拖动→Moved→600ms 落盘→show_note 用 `last_*`；±2px 与 DPI 相关，需真机 |
| AC-M25-6 | 需运行时**且口径待澄清** | 见 P2-1：实现按 SPEC §4.1“保存点最近屏”，AC 文字写“光标屏” |
| AC-M25-7 | 通过（引用 PM 证据） | `project/STATUS.md:26`：pure 23 + hotkey 2 = **25 passed / 0 failed**（≥20），新增 9 个（`tests/pure.rs:248-469`）；本次未复跑 |
| AC-M25-8 | 通过（引用 PM 证据） | `project/STATUS.md:34`：`npm run build` 12 modules、tsc 无报错；本次未复跑 |
| AC-M25-9 | 需运行时点检 | 五分区切换、预览、自动保存、无 JS 报错需真机点击；静态上 7 个新元素 id/绑定齐全 |
| AC-M25-10 | 需运行时回归 | 热键/翻译/复制/历史/导入导出的代码路径未被本次 diff 改动（除 `show_note` 定位与 `save_config` 增量字段），建议烟测一遍 |

## 无法静态判断、留给运行时验收的点

1. **去抖窗口内退出应用**：拖动结束 <600ms 内退出/崩溃，最后一次位置不会落盘（去抖固有，SPEC 允许约 600ms）。属可接受行为，验收时不要当回归。
2. **多显示器 / DPI 缩放**：`last_*` 与 `rcWork` 均为物理像素，但窗口在不同 DPI 屏间移动、或显示器热插拔后坐标是否仍“±2px”，只能在真机确认（AC-M25-5/6）。
3. **AC-M25-6 的显示器口径**：如 P2-1，验收前需 PM 明确以 SPEC §4.1 还是 AC 文字为准；两者在“保存点离光标屏很远”时结论不同。
4. **不透明度视觉**：80% 时阴影/圆角/文字的实际观感、以及设置页预览与便签是否一致，需截图确认（`.note-shell` 的 `opacity` 会同时淡化 box-shadow，这是 SPEC 指定方案的固有表现，不是回归）。
5. **设置页自动保存链路**：`scheduleSave` 去抖 + `saveChain` 串行写盘在真实点击（连续拖滑杆、快速切模式）下是否产生可感知延迟/无 JS 报错，需运行时看 DevTools。
6. **构建/测试闸**：本次评审按任务约束未复跑，结论引用 `project/STATUS.md:26,34,40-45`（cargo test 25 绿、release 构建、npm build、PM `accept.ps1` PASS 5/0）；若 T4 需要独立证据，可自行复跑。

---

**评审限制**：仅静态阅读 diff 与 SPEC；未执行应用、未复跑构建/测试；未审查 `project/` 之外的未跟踪文件。评审通过不代表 AC-M25-5/6/9/10 已实测通过，运行时项由 T4 按 `project/ACCEPT.md` 逐条核验。
