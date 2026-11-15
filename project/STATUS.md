# STATUS · M2.5 便签位置与不透明度

- 阶段：**M2.5-f 交付（done）**
- 更新时间：2026-11-15 14:50（Asia/Shanghai）
- Router/PM：lead
- 阻塞：无
- 结论：**PASS**（AC-M25-1..10 全过，静态闸 5/0，评审 P0/P1=0）
- 下一步：用户验收 → 决定是否并入 0.3.0 / 发布安装包

## 进度板

| 任务 | 角色 | 状态 | 证据 |
|---|---|---|---|
| T1 Rust 后端 | rust-backend | done | cargo test 25 passed；tauri build 通过 |
| T2 前端 | frontend | done | npm run build；7 个控件；payload 完整 |
| T3 独立评审 | reviewer2 | done（PASS，P2×2） | `project/REVIEW.md` |
| T4 验收 | PM | done | `project/ACCEPT.md`，AC 10/10 |

## 证据区

### T1 · Rust 后端（owner: rust-backend，14:13）

- 改动：`src-tauri/src/pure.rs`（+204）、`src-tauri/src/lib.rs`（+212）、`src-tauri/tests/pure.rs`（+206）
- 证据：
  - `cd src-tauri; cargo test --release --offline` → `tests/pure.rs: 23 passed` + `tests/hotkey.rs: 2 passed`，**合计 25 / 0 failed**
  - `npm run tauri build -- --no-bundle` → `Built application at: src-tauri/target/release/lingonote.exe`（30.4 MB）
- 备注：既有 MinGW `multiple non-default manifests` 警告仍在，非本次引入。

### T2 · 前端（owner: frontend，14:0x）

- 改动：`src/main.ts`、`src/settings.ts`、`src/styles.css`、`settings.html`
- 证据：
  - `npm run build` → `✓ 12 modules transformed`、`dist/settings.html 16.10 kB`，tsc 严格模式无报错
  - 静态核对：settings.ts 62 个 `el(...)` id 全部存在；`dist` 产物含 `--note-opacity` 与 7 个新 id

### T3 · 独立评审（owner: reviewer2，14:33）

- 结论 **PASS**：P0=0 / P1=0 / P2=2
- P2-1 remember 显示器口径歧义 → 已由 PM 在 SPEC/ACCEPT 统一为「保存点最近屏（无保存点回退光标屏）」
- P2-2 位置落盘与设置保存的极小竞态 → 记为已知限制（见 ACCEPT §4）

### T4 · 验收（PM，14:48）

- `pwsh -NoProfile -File project\accept.ps1` → **PASS 5 / FAIL 0**（npm build / cargo 25 tests / tauri build / 契约静态检查 / 工作区）
- 运行时烟测（CDP，生产构建）：
  - opacity 80 → 便签 computed `0.8`；**热更新**到 90 → 立即 `0.9`
  - anchor bottom-right + 24px → rect 右/下边距 24/24
  - anchor top-left + 24px → rect (24,24)
  - remember：拖到 (300,200) → 隐藏再显示 0px 误差
  - 屏外 (5000,5000) → 夹回 (1500,680)
  - 设置页五分区切换正常、`window.__errs` 为空、自动保存落盘
- 用户 config 已从备份原样恢复（哈希 `79772026…`），日常安装版 0.2.0 已重新启动。

## 变更记录

- 13:55 PM 完成 SPEC / PLAN / TASKS 冻结。
- 14:05 并行派工 T1/T2（共享任务板 task-1/task-2）。
- 14:16 PM 集成闸通过（5/0）。
- 14:33 评审 PASS（P0/P1=0）。
- 14:48 验收 PASS（10/10），config 恢复，安装版重启。
