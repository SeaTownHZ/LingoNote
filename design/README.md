# LingoNote 图标

产品名：**LingoNote**（中文副名：语签）  
概念：便签 + 对话气泡。便签代表译文，气泡代表语言与交流。

## 配色

| 用途 | 色值 |
| --- | --- |
| 背景深青（上） | `#0E3B3A` |
| 背景深青（下） | `#1E6B62` |
| 便签奶油白 | `#FFF8E7` |
| 便签折角 | `#EAD9B8` |
| 文字/线条 | `#244A46` |
| 气泡珊瑚橙 | `#F2A365` |

## 文件

| 文件 | 用途 |
| --- | --- |
| `lingonote-1024.png` | 主图，用于生成全套应用图标 |
| `lingonote-512.png` / `lingonote-256.png` | 预览与文档 |
| `lingonote.ico` | Windows 多尺寸图标（16–256） |
| `tray-black-44.png` / `tray-black-22.png` | 浅色托盘 |
| `tray-white-44.png` / `tray-white-22.png` | 深色托盘 |
| `icon-preview.png` | 尺寸与托盘效果预览 |
| `make_icon.py` | 生成脚本（Pillow） |

## 生成 Tauri 图标

在项目根目录执行：

```bash
cargo tauri icon design/lingonote-1024.png
```

会生成 `src-tauri/icons/` 下的 PNG、ICO、ICNS 等全套图标。macOS 的 `.icns` 需要在 macOS 或 CI 上生成。

## 备注

- 16px 下细节会丢失，托盘使用单独的简化单色图标（44/22px）。
- 图标草案仍可迭代；当前版本优先保证 32px 以上清晰、托盘轮廓可辨。
