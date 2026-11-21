# 03 · 屏幕捕获

## 1. 能力矩阵

| 能力 | macOS 13+ | Windows 10 2004+ | Linux X11 | Linux Wayland |
| --- | --- | --- | --- | --- |
| 全屏抓帧 | ScreenCaptureKit | Windows.Graphics.Capture | XShm | portal（单次，慢） |
| 单窗口抓帧 | SCK `desktopIndependentWindow` | WGC `CreateForWindow` | XComposite | portal（不支持） |
| 遮挡下抓窗口 | ✅ | ✅ | ❌ | ❌ |
| 帧率（连续） | 60 fps | 60 fps | 30–60 fps | 不支持 |
| 光标 | 可合成 | 需自绘 | 可合成 | 不可 |
| 权限 | 屏幕录制（TCC） | 无（WGC） | 无 | portal 弹窗 |

平台优先级：**macOS 先做通**（开发机），Windows 紧随（用户量最大），Linux X11 兜底，
Wayland 明确降级（只做单次全屏 + 手动滚动）。

## 2. 统一抽象

```rust
pub trait CaptureBackend: Send {
    fn capability(&self) -> Capability;
    fn monitors(&self) -> Result<Vec<Monitor>>;
    /// 抓一个显示器的当前画面（物理像素，sRGB，BGRA 或 RGBA）。
    fn grab_display(&mut self, id: MonitorId) -> Result<Frame>;
    /// 抓一个窗口的内容，包括被遮挡的部分（能力不足时返回 Unsupported）。
    fn grab_window(&mut self, id: WindowId) -> Result<Frame>;
    /// 光标下/指定点的窗口与元素，用于「自动选区」。
    fn probe(&mut self, at: Point) -> Result<ElementRef>;
}

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    /// 统一为 RGBA8，小端，预乘 alpha 一律为 false。
    pub pixels: Vec<u8>,
    pub scale_factor: f32,
    pub captured_at_ms: u64,
}
```

- 所有后端输出**统一成 RGBA8 sRGB**；平台特例（BGRA、P3、HDR）在各自 crate 内转换，
  上层看到的永远是同一种格式。
- `Frame` 不实现 `Clone` 的隐式拷贝；大帧一律 `Arc<Frame>` 传递。

## 3. macOS

> 实现状态（M0）：先用 `CGDisplayCreateImage` + `CGBitmapContext` 转 RGBA8/sRGB，
> 单帧约 200 ms（双屏串行）。它在 macOS 15 上已不可靠，迁移到 SCK 排在 M4 之前。

### 3.1 抓帧

- `SCShareableContent.current` 枚举显示器与窗口；
- 全屏：`SCScreenshotManager.captureImage(contentFilter:configuration:)`（macOS 14+），
  或 `SCStream` 单帧（13）；
- 窗口：`SCContentFilter(desktopIndependentWindow:)` —— 这是**窗口级滚动截屏的基础**，
  遮挡不影响内容；
- 配置：`showsCursor = false`（光标我们自己画，避免重复），`captureResolution = .best`，
  `colorSpaceName = .sRGB`。

### 3.2 权限

```
启动 → CGPreflightScreenCaptureAccess()
      ├─ true  → 正常
      └─ false → 显示引导页（含一张示意图 + 「打开系统设置」按钮）
                 → CGRequestScreenCaptureAccess()
                 → 用户设置后需重启进程（macOS 限制），提示「已完成，请重新打开 PinShot」
```

- 自动滚动需要**辅助功能**权限（`AXIsProcessTrustedWithOptions`），单独引导，按需申请
  （只在用户第一次点「自动滚动」时问，不在启动时打扰）。
- 两项权限都要在设置里显示实时状态，并提供「重新检测」。

### 3.3 窗口探针

- `AXUIElementCopyElementAtPosition` 拿到 UI 元素 → `AXUIElementGetPid` → `NSRunningApplication`
  拿 app 名与 bundle id；
- 元素层级用于「父/子元素」逐级选择（`spec` 里的 SCR 需要它来定位滚动容器）；
- 只能拿到**辅助功能已授权**的 app 的元素；未授权时退化为窗口级矩形。

## 4. Windows

- `Windows.Graphics.Capture`（WinRT）：
  - 显示器：`GraphicsCaptureItem.CreateFromDisplayId`；
  - 窗口：`CreateFromWindowId`（可抓遮挡内容，DXGI 桌面复制做不到这一点）；
  - `FramePool` 复用，避免每帧分配；
- 回退：`BitBlt(GetDC(NULL))` + `CAPTUREBLT`（仅全屏，慢 20–40 ms，用于老系统）；
- DPI：manifest 声明 `PerMonitorV2`，并在每个窗口 `WM_DPICHANGED` 后更新 `scale_factor`；
- 滚动注入：`SendInput` 发 `MOUSEEVENTF_WHEEL`（`WHEEL_DELTA = 120`），
  步长按行数换算（浏览器一行 ≈ 100 px，取 `0.7 × 视口高` 最稳）；
- 元素探针：UI Automation `ElementFromPoint`。

## 5. Linux

- X11：`XShmGetImage`（全屏）、`XCompositeNameWindowPixmap`（窗口，仅未最小化时）；
- 滚动注入：`XTestFakeButtonEvent`（button 4/5）；
- Wayland：`org.freedesktop.portal.Screenshot` 单次全屏，无窗口级、无连续帧 →
  **自动滚动在此平台关闭**，只保留「手动滚动 + 用户自己截的帧」；
- 打包：AppImage，依赖 `libxcb`、`libXcomposite`、`libXtst`。

## 6. 多显示器与虚拟桌面

```
virtual desktop（物理像素）
┌─────────────┬──────────────┐
│ 0,0         │ 2560,0       │   ← 两块屏，scale 2.0 与 1.0
│  2560×1440  │  1920×1080   │
└─────────────┴──────────────┘
```

- 覆盖层为每个显示器建一个窗口（不是一个大窗口），因为 macOS 不允许单窗口跨屏时正确处理
  不同 backing scale；
- 跨屏选区：`union` 所有被覆盖的显示器 → 按虚拟桌面坐标裁剪每块屏幕的位图 → 横向/纵向拼接；
- 跨屏时若两块屏 scale 不同，先统一到**较高**的 scale 再拼，避免一半模糊；
- 导出时记录 `scale_factor`，保证 2x 屏导出的是 2 倍像素图。

## 7. 色彩

- 默认把显示器色彩空间转换到 **sRGB**（`CGColorSpaceCreateWithName(kCGColorSpaceSRGB)`）；
- macOS 上 P3 屏截图若不做转换，UI 里看到的高饱和色会与「复制到其它程序」的颜色不一致；
- HDR 屏：`SCStreamConfiguration.pixelFormat` 用 10-bit 时先 tone map 到 8-bit sRGB
  （简单 Reinhard，不做逐显示器校准），并在设置里提供「保持 HDR（导出 16-bit PNG）」的实验开关；
- 取色器永远报 sRGB 值，并在工具提示里标注色域。

## 8. 性能预算

| 场景 | 预算 | 说明 |
| --- | --- | --- |
| 单块 5K 屏抓帧 | ≤ 35 ms | SCK 冷启动约 100 ms，常驻 stream 可降到 12 ms |
| 4 块屏预热后抓帧 | ≤ 60 ms 总计 | 并行抓 |
| 覆盖层上屏（14.7 MP） | ≤ 40 ms | `ImageBitmap` + `drawImage` |
| 每帧内存 | 4 B/px | 5120×2880 ≈ 59 MB；多屏时按需保留，非活动屏可降采样 |

- 常驻模式下持有 `SCStream` 会持续耗电，因此**只在滚动任务期间**保活 stream，
  其余时间走一次性截图 API；
- 内存上限：同时保留的 `Frame` 不超过 3 个全屏位图；历史帧落盘后释放。
