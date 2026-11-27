# Windows 支持与打包

**结论**：同一份源码现在两边都能打。macOS 用 CoreGraphics/AppKit；Windows 用 GDI +
SendInput，截图、标注、导出、贴图、历史、设置、手动滚动截屏在 Windows 10/11 x64 上可用。

> 📦 **当前 Windows 包**：`PinShot_0.1.0_x64-setup.exe`（安装版）和 `PinShot-portable.exe`
> （免安装版）已于 2026-11-27 更新，可从 [GitHub Release](https://github.com/full-coder/pinshot/releases/tag/v0.1.0) 下载。
> `docs/PinShot-Windows-0.1.0/` 保留的是 2026-11-13/14 的旧构建，请勿与当前包混用。
> 本机是 macOS，无法运行 Windows 包复验；下列 Windows 真机记录来自此前验证。

| 功能 | macOS | Windows |
| --- | --- | --- |
| 截图 / 冻结帧覆盖层 / 多屏 / 高 DPI | ✅ | ✅ |
| 框选、标注、撤销、放大镜 | ✅ | ✅ |
| Enter 复制、⌘S / Ctrl+S 保存 | ✅ | ✅ |
| 贴图窗口 + 滚轮缩放 + 双击关闭 | ✅ | ✅ |
| 历史记录、设置、中英文 | ✅ | ✅ |
| 滚动截屏（用户自己滚、程序抓帧） | ✅ | ✅ |
| 滚动截屏（程序自动滚） | ✅ Accessibility | ❌ 未移植 |
| OCR（提取文字） | ✅ Vision | ❌ 未移植，会提示仅 macOS 支持 |
| 开机启动 | LaunchAgent | `HKCU\...\CurrentVersion\Run` |

最新安装版和免安装版见 [GitHub Release](https://github.com/full-coder/pinshot/releases/tag/v0.1.0)；
`docs/PinShot-Windows-0.1.0/` 保留 2026-11-13/14 的旧构建和当时的源码。

## 构建

### Windows

1. 一次性装好：
   - Rust（stable，`x86_64-pc-windows-msvc`）
   - Visual Studio Build Tools，勾选「使用 C++ 的桌面开发」（MSVC 链接器 + Windows SDK）
   - Node.js 22+、pnpm
   - WebView2 Runtime（Win11 自带；Win10 一般已随 Edge 安装）
2. 在项目根目录：

   ```powershell
   pnpm install
   pnpm tauri build --bundles nsis
   ```

   `tauri.conf.json` 的 `beforeBuildCommand` 会自己跑 `pnpm build`，不需要先生成 `dist`。
   Tauri 打包器会从 GitHub 下载 NSIS；网络不稳时：

   ```powershell
   $env:TAURI_BUNDLER_TOOLS_GITHUB_MIRROR="https://gh-proxy.com"
   ```

   MSI 还要额外下载 WiX 3.14；没有 WiX 就只打 NSIS，功能一样。

3. 产物：

   ```
   src-tauri\target\release\bundle\nsis\PinShot_0.1.0_x64-setup.exe
   src-tauri\target\release\PinShot.exe          # 免安装
   ```

   `mainBinaryName` 只在 `tauri.windows.conf.json` 里设置，所以 Windows 上程序叫
   `PinShot.exe`，macOS 的包内可执行文件仍是 `pinshot-desktop`，打包脚本不受影响。

4. 安装包未签名，SmartScreen 会提示「未知发布者」，选「仍要运行」。

### macOS

不受 Windows 代码影响；`pnpm tauri build` / `scripts/make-dmg.sh` 照旧，见 README。

## Windows 实现说明

- **抓屏**：`crates/pinshot-capture/src/win.rs`。`BitBlt` 桌面 DC + `CAPTUREBLT`，输出
  RGBA；进程 per-monitor DPI aware，`Monitor.bounds` 用逻辑点、`pixel_width` 用物理像素。
- **窗口枚举 / 单窗截图**：`EnumWindows` + DWM 扩展边框；单窗截图取窗口所在的屏幕区域
  （滚动截屏前覆盖层已隐藏，不会把自己截进去）。
- **输入**：`events.rs` 用 `SendInput` 实现点击、拖拽、滚轮、按键；全局快捷键走 Tauri 插件。
- **托盘**：Windows 托盘用彩色 `icon.png`（macOS 的 `tray.png` 是模板图标，在深色任务栏上会消失）。
- **开机启动**：`reg add HKCU\Software\Microsoft\Windows\CurrentVersion\Run`，值里带
  `--autostart` 便于诊断；每次启动都会按配置重写（自愈）。登录时 Explorer 可能还没准备好
  通知区域，托盘创建失败会重试 0.8 / 1.6 / 3.2 秒，不再让进程退出。
- **单实例**：`tauri-plugin-single-instance`；重复启动时第二个进程退出，已在运行的那个
  打开设置窗口。
- **数据目录**：`%LOCALAPPDATA%\ai.pinshot.desktop`；默认保存目录
  `%USERPROFILE%\Pictures\PinShot`。
- **诊断日志**：Windows 上写到 `%TEMP%\pinshot-diag.log`；macOS 仍是 `/tmp/pinshot-diag.log`，
  现有排查脚本不用改。
- **配置里的路径**：`config.toml` 会转义反斜杠，`C:\Users\…` 既是本程序能读的，也是合法 TOML。

## 已知的 Windows 细节

- 覆盖层是每屏一个无边框置顶窗口，任务栏也在覆盖范围内。
- 贴图窗口先创建、再隐藏覆盖层：反过来 WebView2 偶尔会停在白色首帧。
- `--text-selftest` / `--pin-selftest` 里 `new KeyboardEvent` 合成的回车在 WebView2 下不一定
  触发 React 的输入法提交；真实键盘输入正常。
- 高 DPI 混合缩放（100% + 200% 两块屏）下坐标按显示器分别换算；单屏 200% 已实测通过。

## 验证记录

- Windows 11 + WebView2 实机：F1、框选、标注、复制、保存、贴图缩放、历史、设置中英文、
  手动滚动截屏（2000×3000）全部通过；安装/静默安装/卸载均正常。
- 本仓库回归：macOS 上 `cargo test --workspace`、`pnpm test`、`tsc`、`clippy` 全绿；
  `--pin-selftest` 贴图窗口正常（zoom 1400×900 → 1676×1078），`--probe-selftest` 首帧 92 ms。
- Windows 后端在本机做了交叉检查：
  `cargo check -p pinshot-capture --target x86_64-pc-windows-msvc` 与同 target 的 `clippy`
  无 warning；完整 Windows 打包仍需在 Windows 机器上跑（MSVC 链接器只有 Windows 有）。
