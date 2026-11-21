# 02 · 架构

## 1. 总体形状

```
┌──────────────────────────────────────────────────────────────┐
│  app（Tauri 2 壳，src-tauri/）                                 │
│  · 窗口：overlay / toolbar / pin-* / history / settings       │
│  · 只做 IPC 适配与窗口生命周期，不含业务逻辑                    │
└───────────────┬──────────────────────────────────────────────┘
                │ commands（typed） / events（channel）
┌───────────────▼──────────────────────────────────────────────┐
│  pinshot-app（Rust，无 GUI 依赖）                              │
│  · Session 状态机：Idle → Overlay → Selecting → Annotating →   │
│    Exporting；Scrolling 是 Annotating 的子状态                 │
│  · 快捷键路由、工具条模型、撤销栈、导出管线                     │
└──┬────────┬────────┬────────┬────────┬────────┬──────────────┘
   │        │        │        │        │        │
┌──▼──┐ ┌───▼───┐ ┌──▼────┐ ┌─▼─────┐ ┌▼──────┐ ┌▼─────────┐
│core │ │capture│ │stitch │ │annot  │ │store  │ │ocr(P2)   │
│几何 │ │后端   │ │滚动拼接│ │场景图 │ │SQLite │ │文字识别  │
│色彩 │ │三平台 │ │纯函数  │ │渲染   │ │blob   │ │          │
└─────┘ └───────┘ └───────┘ └───────┘ └───────┘ └──────────┘
```

依赖方向严格单向：`core` 不依赖任何其他 crate；`capture` / `stitch` / `annot` / `store`
只依赖 `core`；`pinshot-app` 依赖全部；`cli` 与 `app` 平级，都只调用 `pinshot-app` 的公开 API。

## 2. crate 划分

| crate | 职责 | 不做 |
| --- | --- | --- |
| `pinshot-core` | 几何（Rect/Point/Size、物理像素与 dip 换算）、颜色、错误类型、配置模型、图像基本操作（crop/scale/blend） | 任何平台 API、任何 IO |
| `pinshot-capture` | `CaptureBackend` trait + macOS / Windows / Linux 实现；显示器与窗口枚举；权限查询 | 拼接、标注、存盘 |
| `pinshot-stitch` | 滚动拼接纯函数：位移估计、固定行/列掩码、合成、置信度 | 抓帧、滚动事件、UI |
| `pinshot-annotate` | 场景图模型（Shape/Transform/Style）、命中测试、撤销栈、光栅化（马赛克/模糊/文字） | 交互（谁选中谁由 app 决定） |
| `pinshot-store` | SQLite 历史索引、内容寻址 blob、设置持久化、保留策略 | 图片处理 |
| `pinshot-app` | Session 状态机、快捷键、导出管线、贴图窗口模型（不含窗口创建） | 直接创建窗口 |
| `pinshot-cli` | `snip` / `scroll` / `paste` / `history` / `bench` | 任何 GUI 假设 |
| `src-tauri` | Tauri 命令、窗口创建、托盘、全局热键、权限申请 | 业务判断 |

## 3. 关键决策

### ADR-1：冻帧（freeze-frame）架构

进入截图模式时，一次性把每个显示器抓成一张位图，覆盖层只显示这张位图。

**为什么**：
- 抓帧只需要 10–30 ms，之后的所有交互（框选、放大镜、标注）都在静态图上，
  延迟与帧率完全由 UI 层决定，不受屏幕内容变化影响；
- 覆盖层不需要「透明但可见」的实时合成，避免了透明窗口 + 实时抓屏的撕裂和色偏；
- 用户看到的颜色与最终导出的像素**逐位相同**，取色器天然准确；
- 单测可以直接喂位图，不需要真屏幕。

**代价**：屏幕在截图期间变了，用户看到的是旧画面 → 用 `R`（刷新截图）补偿，并在页面上有
视频/动画时提示「画面已定格」。

### ADR-2：坐标为王

- 所有几何量默认**物理像素**（`Rect { x, y, w, h }`，整数）。
- `dip`（逻辑像素）只在两处出现：UI 布局与热键提示；换算集中在 `pinshot-core::scale`。
- 每个显示器带 `scale_factor`，跨屏选区先换算到虚拟桌面的物理坐标系再裁剪。
- 标注形状存物理像素坐标，导出时按目标缩放重采样（1x/2x），不做「先缩后画」。

### ADR-3：滚动拼接是纯函数

`stitch(frames: &[Frame], opts: &StitchOptions) -> Result<Stitched, StitchError>`

- 不读屏幕、不碰文件、不碰时钟；所有时间戳由调用方传入（用于日志）。
- 因此它能在 CI 里跑合成帧测试，也能在 CLI 里用「导入的帧序列」复现线上问题。
- 抓帧与滚动策略属于 `pinshot-app::scrolling`，可以单独替换（自动 / 手动）而算法不变。

### ADR-4：UI 用 Web 技术，但只在覆盖层

覆盖层是一个覆盖全屏的无边框窗口，背景是冻帧位图。标注绘制在 `<canvas>` 上：
- 底图用一次 `putImageData` / `ImageBitmap` 上屏；
- 标注层按需重绘（脏矩形 + `requestAnimationFrame`）；
- 交互层是 DOM（工具条、尺寸标签），保证可访问性与输入框行为。

不需要 WebGL；Canvas2D 在 5K 全屏（14.7 MP）下画一条线远低于 1 ms。

**只有一个渲染器（实现后修正）**：标注的场景图与绘制都在前端（`src/scene.ts` /
`src/render.ts`），导出时用同一份绘制代码把「底图 + 标注」渲染成 PNG，再交给 Rust 编码、
写剪贴板、落盘。原来的设计是 Rust 也要有一份渲染器，实现时否掉了：

- 两份渲染器必然漂移，而截图工具的底线是「所见即所得」；
- 文字要正确处理 CJK，浏览器已经具备完整的字体管理，Rust 侧则要自带字体与排版；
- 马赛克/模糊在 Canvas2D 上各 10 行（`filter: blur()` / 缩放在画回去），
  而 Rust 侧需要 tiny-skia + 手写区域采样。

代价：命令行不能给图片加标注（目前也不需要），以及导出多一次 PNG 编解码（5K 约 120 ms，
只在有标注时发生；无标注仍走 Rust 直接裁剪的快速路径）。

### ADR-5：为什么不是 Electron / Qt

| | Tauri（选定） | Electron | Qt |
| --- | --- | --- | --- |
| 常驻内存 | 40–80 MB | 150–250 MB | 40–70 MB |
| 覆盖层透明/置顶控制 | 需要少量平台代码 | 容易 | 容易 |
| 与 Rust 核心同进程 | 是 | 需要 N-API | 需要绑定 |
| 打包体积 | ~10 MB | ~120 MB | ~30 MB（含 Qt） |
| 团队熟悉度 | 高（与 OpenIDEA 一致） | 高 | 低 |

屏幕捕获、滚轮注入、窗口枚举这些「必须原生」的部分本来就要写平台代码，
所以框架省下的那点工作量不是决定因素；内存与打包体积才是。

## 4. 线程与进程模型

```
main (UI)        ── 事件循环、覆盖层渲染
capture thread   ── 抓帧（阻塞式平台 API），只做抓帧，做完即返回
stitch worker    ── rayon 池，滚动拼接 + 马赛克/模糊；不阻塞 UI
writer thread    ── PNG 编码 + blob 落盘 + SQLite 写入（单写者，串行）
scroll injector  ── 自动滚动的定时器线程（每 30–120 ms 一次，可取消）
```

规则：
- 任何 > 8 ms 的工作不得在 UI 线程执行（抓帧、编码、模糊、SQLite 写）。
- 所有长任务接收 `CancellationToken`（Esc、选区变化、窗口关闭都要能取消）。
- `Store` 单连接、单写者；读可以并发（WAL + 每次操作新建只读连接）。

## 5. IPC 契约

命令（前端 → Rust）：

| 命令 | 参数 | 返回 |
| --- | --- | --- |
| `session_start` | `{ mode: "snip" \| "scroll" \| "paste" }` | `{ session_id, frames: [FrameRef], monitors: [Monitor] }` |
| `session_region` | `{ session_id, region }` | `{ hit: ElementRef \| null }` |
| `session_annotate` | `{ session_id, ops: [ShapeOp] }` | `{ revision }` |
| `session_export` | `{ session_id, target: "clipboard" \| "file" \| "pin", path? }` | `{ capture_id, width, height }` |
| `scroll_start` | `{ session_id, mode: "auto" \| "manual", bounds }` | `{ job_id }` |
| `scroll_frames` | `{ job_id, since: u64 }` | `{ frames: [FramePreview], composite_height, state }` |
| `pin_create` | `{ capture_id, at }` | `{ pin_id }` |
| `pin_update` | `{ pin_id, patch }` | `{ ok }` |
| `history_query` | `{ cursor, limit, filter }` | `{ items, next_cursor }` |

事件（Rust → 前端，`Channel`）：

| 事件 | 负载 |
| --- | --- |
| `scroll:progress` | `{ job_id, frames, height, confidence, state }` |
| `scroll:seam_warning` | `{ job_id, at_y, score }` |
| `session:permission` | `{ kind: "screen" \| "accessibility", granted }` |
| `pin:closed` / `pin:destroyed` | `{ pin_id }` |
| `toast` | `{ level, text }` |

约束：
- 所有坐标 0-based、物理像素、`i32`；尺寸 `u32`。
- 帧数据不通过 JSON 传，用 `tauri::ipc::Response` 的 raw bytes + 独立元数据。
- 每个命令返回 `Result<T, PinshotError>`，`PinshotError { kind, message, hint? }`，
  `kind ∈ { Permission, Unsupported, NotFound, Busy, Io, Internal }`。

## 6. 状态机

```
Idle ──hotkey──▶ Freezing ──frames──▶ Selecting
                                        │ drag
                                        ▼
                                   RegionSet ──tool──▶ Annotating
                                        │                 │
                                        │                 ├─ S ──▶ Scrolling ──done──▶ Annotating(长图)
                                        │                 │
                                        ▼                 ▼
                                     Exporting ◀──── copy/save/pin
                                        │
                                        ▼
                                      Idle
Esc 从任意状态回到上一稳定态；Freezing 失败 → Idle + toast。
```

`Scrolling` 是 `Annotating` 的子状态：滚动期间禁止绘制，完成后把长图作为新的底图继续标注。

## 7. 错误与可观测性

- 所有错误携带 `kind` 与一句可执行提示（「需要在系统设置里允许屏幕录制」而非「CGDisplayStream 失败 -3801」）。
- 日志写 `~/Library/Logs/PinShot/`（Win: `%LOCALAPPDATA%\PinShot\logs`），按 10 MB 轮转，保留 3 份。
- 崩溃时写一份最小 dump 摘要（版本、平台、最近 20 条日志），保存在本地，**不上传**。
- `pinshot doctor` 输出：平台版本、权限状态、显示器列表、后端能力矩阵、配置路径。

## 8. 配置

`~/Library/Application Support/ai.pinshot.desktop/config.toml`（Windows/Linux 对应目录）：

```toml
[general]
theme = "auto"
launch_at_login = false
language = "zh-CN"

[hotkeys]
snip = "F1"
paste = "F3"
hide_all_pins = "Shift+F3"

[capture]
include_cursor = false
dim_outside = 0.5
magnifier_zoom = 8

[scroll]
mode = "auto"            # auto | manual
step_ratio = 0.7
settle_ms = 120
max_height = 20000
confidence = 0.85

[history]
keep_days = 30
keep_items = 1000
max_size_mb = 2048

[export]
format = "png"           # png | webp | jpg
quality = -1             # -1 自动
quick_save_dir = "~/Pictures/PinShot"
```

非法值：回落默认 + 一条 `toast` 警告，绝不因配置错误启动失败。
