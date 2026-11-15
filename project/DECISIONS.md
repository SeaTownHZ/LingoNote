# DECISIONS · M2.5

记录本里程碑的关键取舍，后续会话以此为准，不再反复讨论。

| 编号 | 决策 | 理由 | 代价/备注 |
|---|---|---|---|
| D1 | `schema_version` 保持 2，只做增量字段 | 新增字段全部有默认值，旧配置可读；避免迁移与导入校验连锁改动 | 语义上属 v2.5，文档里标注即可 |
| D2 | 不透明度存整数百分比（80–100），不存 0.8 浮点 | 设置页滑块天然整数；钳制与展示简单；JSON 可读 | 与 PRD 草案的 1.0 浮点不同，PRD 以本 SPEC 为准 |
| D3 | 位置配置放独立 `window` 段，不塞进 `behavior` | 位置是窗口行为、有自动写入的 `last_x/y`；与用户手动配置解耦，便于保存保留策略 | `ConfigInput.window` 需 `serde(default)` 兼容旧前端 |
| D4 | 记住位置：Rust 监听 `Moved` 去抖 600ms 落盘，而非前端 IPC | 不新增命令、不改前端事件模型；拖动结束自然收敛 | 程序化定位也会触发一次 Moved，写入同值，可接受 |
| D5 | 不透明度用 CSS 变量作用于 `.note-shell`，不用 Win32 分层窗口 alpha | 跨平台一致、随主题/阴影自然组合、不碰 Win32 互操作风险 | 便签内容整体变透明（符合"整张便签"语义） |
| D6 | 记住的是绝对屏幕坐标，不是"某显示器+锚点" | 实现简单、多屏场景可预测（夹回保存点所在屏） | 换屏后可能夹回，用户可重新拖动 |
| D7 | 显示器选择沿用"光标最近"；remember 用保存点最近 | 跟随鼠标是默认行为；remember 的语义是"回到原处" | 保存点跨屏时有 MonitorFromPoint 兜底 |
| D8 | 评审用独立子代理读 diff，不启用 code_review 工具 | 用户未显式要求代码审查工具授权；子代理评审同样独立且不写实现文件 | 后续可在 PM 流程里把 code_review 作为可选闸 |
| D9 | 生产构建一律走 `npm run tauri build -- --no-bundle`，禁用裸 `cargo build --release` | 实测裸 cargo 构建的 release 二进制仍使用 `devUrl`（127.0.0.1:1420），运行后白屏；Tauri CLI 才会注入生产配置并嵌入 frontendDist | 验收脚本 `project/accept.ps1` 已改为 Tauri CLI 构建；构建慢约 1 分钟 |

## 待用户确认

- SPEC 第 3 节配置字段命名（`position_mode` / `anchor` / `margin_x` / `margin_y`）是否接受；不接受在 M2.5-b 之前改，改则需双方同步。
