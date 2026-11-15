# PLAN · M2.5 便签位置与不透明度

## 里程碑

| 阶段 | 交付物 | 验收 |
|---|---|---|
| M2.5-a 契约冻结 | `project/SPEC.md`（配置 schema、纯函数签名、设置项 id） | 用户确认 SPEC |
| M2.5-b 并行实现 | T1 Rust 后端 / T2 前端 | 各自 DoD 证据 |
| M2.5-c 集成 | 合并工作区，`npm run build` + `cargo test` | 全绿 |
| M2.5-d 评审 | `project/REVIEW.md` | P0/P1 问题清零 |
| M2.5-e 验收 | `project/ACCEPT.md` + 运行时烟测 | AC-M25-1..10 全过 |
| M2.5-f 交付 | git 提交 + 0.2.5 构建产物 | 里程碑提交 |

## 角色与派工

| 角色 | 任务 | 写入域 | 模型档位 |
|---|---|---|---|
| Rust 实现 | T1 | `src-tauri/src/pure.rs`、`lib.rs`、`tests/pure.rs` | 默认 |
| 前端实现 | T2 | `src/main.ts`、`src/settings.ts`、`src/styles.css`、`settings.html` | 默认 |
| 独立评审 | T3 | `project/REVIEW.md` | 默认（只读 diff） |
| 验收 | T4 | `project/ACCEPT.md`、`STATUS.md` | PM 本人 |

- T1、T2 无依赖，可并行；契约见 SPEC 第 3、4 节，双方不得自行改契约。
- T3 依赖 T1+T2 完成（`blocked_by`），T4 依赖 T3。
- 同一文件同一时间只有一个 owner；越界改动一律打回。

## 执行顺序

1. PM 建共享任务板 T1–T4（带写入域与依赖）。
2. 并行启动 T1 / T2。
3. 双方回报证据 → PM 跑集成构建（`npm run build` + `cargo test`）。
4. 启动 T3 评审；P0/P1 问题回给对应 owner 修复，复审。
5. T4 验收：静态检查 + 运行时 CDP 烟测（设置页 DOM、便签透明度/位置、config 落盘）。
6. 更新 `STATUS.md`，git 提交，向用户汇报。

## 风险与对策

| 风险 | 影响 | 对策 |
|---|---|---|
| Win32 多屏坐标/DPI 差异 | 位置算错 | 纯函数用 rcWork 物理像素；remember 夹回工作区；单测覆盖翻转/越界 |
| 拖动时频繁写配置 | 磁盘抖动/配置竞态 | 600ms 去抖；原子写；只改 last_x/y |
| 设置页自动保存清空 last_x/y | 记住位置失效 | `save_config` 在入参为 None 时保留磁盘旧值（SPEC 3） |
| 便签透明度影响流式文字可读性 | 体验下降 | 下限 80%；只作用于 `.note-shell` |
| 前后端契约漂移 | 编译/运行失败 | SPEC 冻结字段名；类型两处逐字一致；构建闸 |
| 自动保存与位置保存并发写 | 丢字段 | 原子写 + 读-改-写；验收时检查字段完整性 |

## 试点说明

这是 Router-PM 工作流的第一次真实试点：PM 只写 SPEC/任务图/闸门，实现与评审由子代理完成；
用户只在"SPEC 确认"和"最终验收"两处介入。
