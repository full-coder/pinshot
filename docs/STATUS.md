# 当前状态

PinShot 是本地优先的截图 / 标注 / 贴图 / 滚动截屏工具，macOS 与 Windows 10/11 x64 均可构建。
这一份只记当前验证结论和关键设计，过程细节看 git 提交。

## 验证

| 项 | 结果 |
| --- | --- |
| Rust | `cargo test --workspace` → 83 passed；`cargo clippy --workspace --all-targets -- -D warnings` 0 warning |
| 桌面壳 | `cargo test --manifest-path src-tauri/Cargo.toml` → 2 passed；同 target clippy 0 warning |
| 前端 | `pnpm vitest run` → 27 passed；`pnpm exec tsc --noEmit` 通过 |
| 截图首帧 | `--capture-selftest 2`：首帧 107 ms / boot 94 ms（2880×1800 Retina，稳态） |
| 滚动拼接 | `--scroll-selftest window:<演示页>,manual`：40 帧拼成 2400×6168 |
| 贴图 | `--pin-selftest`：贴图窗落在选区原位；右键菜单中文「复制图片到剪贴板 / 关闭贴图」，点关闭生效 |
| 设置 | `--window-close-selftest`：settings window closed ok |

## 关键设计

1. **单一渲染器**（`src/render.ts`）：预览和导出走同一份 Canvas2D 代码，所见即所得，CJK 文字用系统字体。Rust 只负责裁剪、编码、剪贴板、落盘。
2. **冻帧覆盖层**：先抓一帧静止画面再显示窗口，标注画在这一帧上，滚动截屏时才隐藏它。
3. **暖覆盖层**：窗口启动时预建并隐藏，页面常驻；下次截图只发一个事件，省去页面加载时间。默认常驻（约 80–100 MB），可在设置里关掉「保留预热窗口」改成空闲 60 秒后释放。

## 近期变更

- **v0.1.0**：重置为单个初始提交，打 tag 并发布。
- 贴图窗落在选区原位（`set_pin_origin` + 显示器全局原点），不再固定到左上角；贴图窗右键改为跟随语言的中文菜单（`pin_copy` / 关闭），并修掉背景透明、点关闭无效。
- README 开头加功能海报；安装说明改为从 GitHub Release 下载 + 从源码构建。
- Windows 滚动截屏提示改用 `tauri-plugin-notification`（仅交叉编译验证，真机通知需 Windows 实机确认）。
- 导出 PNG 改用 `Arc<[u8]>`，复制 / 保存 / 贴图 / 历史共享同一份字节。
- 体检修复一批问题：设置关不上、Esc/Enter 双触发、滚动取消竞态、配置原子写、历史字段钳制、长图超视口回滚、AX 引用泄漏、Windows 并行抓屏等。
- 滚动截屏 Enter 回归修复：re-show 只看 serial，不被 `scroll_cancel` 误判。

## 平台差异

macOS 与 Windows 的截图、标注、导出、贴图、历史、设置、手动滚动截屏都可用。Windows 上 OCR 与自动滚动未移植（OCR 会提示仅 macOS 支持），程序未签名。细节见 [docs/WINDOWS.md](WINDOWS.md)。
