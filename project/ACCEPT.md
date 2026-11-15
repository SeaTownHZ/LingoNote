# ACCEPT · M2.5 便签位置与不透明度

- 验收时间：2026-11-15 14:48（Asia/Shanghai）
- 验收者：Router/PM（lead，非实现者）
- 评审：T3 独立子代理 PASS（P0=0 / P1=0 / P2=2），见 `project/REVIEW.md`
- 构建：`npm run tauri build -- --no-bundle`（Tauri CLI 生产构建，嵌入 frontendDist）
- 结论：**PASS（AC-M25-1..10 全过）**，可交付里程碑提交

## 1. 静态闸（project/accept.ps1）

| 检查 | 结果 | 说明 |
|---|---|---|
| 前端构建 npm run build | PASS | `✓ 12 modules transformed`，tsc 严格模式无报错 |
| Rust 测试 cargo test | PASS | **25 passed / 0 failed**（pure 23 + hotkey 2，含新增 9 个位置/Ui 用例） |
| 生产构建 tauri build | PASS | `lingonote.exe` 30.4 MB，Tauri CLI 生产构建 |
| 配置契约静态检查 | PASS | 8 项关键字齐全 |
| 工作区待提交 | PASS | 9 个改动，验收后统一提交 |

> 关键教训（已写入 DECISIONS D9）：裸 `cargo build --release` 产出的二进制仍指向 `devUrl`（127.0.0.1:1420），运行白屏；生产闸必须走 Tauri CLI。首轮验收曾因此失败，已修 `accept.ps1` 后复跑通过。

## 2. 运行时烟测（WebView2 CDP，release 生产构建）

启动方式：`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`，页面 URL 为 `http://tauri.localhost/`（生产资源，非 dev server）。

| AC | 结论 | 实测证据 |
|---|---|---|
| AC-M25-1 旧配置兼容/字段补全 | ✅ | 测试前 `config.json` 无 `window`/`opacity`（备份哈希 `79772026…`）；首次启动后自动补齐 `ui.opacity=100` 与 `window{cursor,bottom-right,16,16,null,null}`，应用正常启动 |
| AC-M25-2 不透明度 80 生效/重启保持/实时 | ✅ | 设置页滑杆置 80 → `config.json ui.opacity=80`；便签页 `getComputedStyle(.note-shell).opacity = "0.8"`、CSS 变量 `--note-opacity=0.8`；截图 `evidence-m25/note-opacity-80-anchor-br.png`。另测：便签显示中把滑杆改 90，**不重启**便签立即变 `0.9` |
| AC-M25-3 anchor bottom-right + 24px | ✅ | 光标屏工作区 1920×1040，便签 420×360；窗口 rect `R=1896, B=1016` ⇒ 右/下边距 **24 / 24** |
| AC-M25-4 九宫格/越界夹回 | ✅ | 单测覆盖 9 个锚点 + 越界；运行时抽验 top-left+24 ⇒ rect `L=24, T=24` |
| AC-M25-5 记住上次位置 | ✅ | 拖到 (300,200) → 去抖后 `config.last=(300,200)`；隐藏再显示 ⇒ rect `(300,200)`，误差 0px |
| AC-M25-6 屏外坐标夹回 | ✅ | 外部写入 last=(5000,5000) → 显示后 rect `(1500,680)`＝工作区右下极限（1920−420, 1040−360），配置自愈为 (1500,680) |
| AC-M25-7 测试 | ✅ | 25 passed（≥20），既有 14 个未回归 |
| AC-M25-8 前端构建 | ✅ | 见静态闸 |
| AC-M25-9 设置页 | ✅ | 7 个新 id 全部存在；五个分区点击切换 `general→service→appearance→advanced→about` 全部命中；`window.__errs` 为空；自动保存（400ms 去抖）把 opacity/模式/锚点/边距写盘；预览 opacity 同步 `0.8` |
| AC-M25-10 回归 | ✅ | 翻译/热键/历史/导入导出逻辑未改（diff 仅涉及配置保存与窗口定位）；设置页与便签正常加载；25 个测试全绿。注：未做真实 API 翻译（避免消耗额度），翻译链路零改动 |

## 3. 环境与清理

- 测试用 release 进程已退出；`config.json` 已从备份原样恢复（哈希 `79772026…` 与测试前一致）。
- 本机日常使用的安装版 LingoNote 0.2.0 已重新启动（PID 10456）。
- 测试期间未写用户凭据、未调用翻译 API、未改注册表/计划任务。

## 4. 已知限制（评审 P2，不阻断）

1. **位置落盘与设置保存的极小竞态**：两者都是"读配置→改→整文件原子写"，极端情况下（拖完 600ms 内正好改设置）可能丢一次更新，下次操作自愈。M2.5 记录为已知限制，后续可加进程内写锁。
2. **remember 的显示器口径**：实现按 SPEC §4.1「保存点最近屏」，无保存点回退光标屏；AC-M25-6 已同步改写，消除歧义。
3. 验收未覆盖：9 个锚点仅抽验 2 个（其余由纯函数单测覆盖）、未做真实 API 翻译、未做 macOS 验证。

## 5. 证据文件

- 截图：`project/evidence-m25/note-opacity-80-anchor-br.png`（80% 不透明、右下角锚点、24px 边距）
- 静态闸：`project/accept.ps1` 运行输出（PASS 5 / FAIL 0）
- 评审：`project/REVIEW.md`
- 进度与证据块：`project/STATUS.md`
