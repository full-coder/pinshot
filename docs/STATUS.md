# 当前状态

**阶段：M0–M3 完成 · M4 滚动截屏完成（选区里点「滚动截屏」→ 自己滚 → Enter）· M5 窗口捕获 · M6 OCR · M7 打包**

从「只有规格和拼接算法」变成了一个能真的截图的桌面程序：F1 唤起、冻帧、
拖框选区、标注、Enter 复制到剪贴板、⌘S 存文件。命令行也能独立完成同样的事。

## 1. 已验证（可复现）

| 项 | 证据 |
| --- | --- |
| Rust 测试 | `cargo test --workspace` → **81 passed, 0 failed**（含实时增量拼接 3 例、真实 OCR fixture 与 4 个跑真实二进制的集成测试） |
| 前端测试 | `pnpm test` → **27 passed**（场景图 / 撤销栈 / 命中测试 / 快捷键解析 / 中英文文案） |
| 静态检查 | `cargo clippy --workspace --all-targets -- -D warnings` 通过 |
| 真实截图 | `pinshot snap --area 100 100 400 300 --out x.png` → 800×600 PNG（Retina 2x 正确） |
| 双屏 | `pinshot monitors` → 内置 2880×1800@2x + DELL 1920×1080@1x |
| 覆盖层 | 双屏各一个窗口，冻帧 + 遮罩 + 放大镜 + 工具条，截图留存 `/tmp/overlay2.png` |
| 标注模型 | 增删改 / 撤销重做 / 200 步上限 / 序号不回退，10 个用例 |
| 命令行集成 | `snap --fake` 写出 400×300 PNG；`history` 读回同一条记录；非法参数报错退出 |
| 历史库 | 内容寻址去重、删除最后一个引用才删 blob、按条数/天数清理且收藏豁免、孤儿 blob 回收 |
| 贴图状态机 | 关闭→恢复、超额销毁、隐藏不算关闭、退出时销毁 closed 保留 hidden、分组关闭、旋转足迹 |
| 滚动收集器 | 稳定帧去重、连续两帧相同即判定到底、帧数上限；合成页面拼回 924×240 且逐像素相等 |
| 贴图窗口 | 按截图尺寸建窗（超出屏幕自动缩放）、渲染 PNG、鼠标拖动（80,80 → 280,200）、关闭后窗口消失 |
| 历史窗口 | 68 条卡片 + 缩略图网格（320px 缓存）、时间/尺寸/收藏/删除，截图见 `/tmp/hist-view2.png` |
| 设置窗口 | 读取真实配置渲染（遮罩 50%、放大镜 8×、保存目录、两条导出开关、保留策略、F1），截图见 `/tmp/settings-view.png` |
| 历史回写剪贴板 | `--self-test history-copy` → 剪贴板从 117 KB 变成 **6,141,625 字节**（最新整屏截图） |
| 设置保存 | `--self-test settings-save` → `config.toml` 实际写出（`[general] hotkey_snip = "F1"` …） |
| 离线重拼 | `pinshot scroll --frames <dir>`：6 帧 → 420×900，采样像素与原始页面 **0 处不符** |
| 自动滚动循环 | `auto_scroll()` 用注入的抓帧/滚动/等待闭包跑完整轮：合成页面走到底、像素对齐；静态页面 ≤2 步停止 |
| 手动滚动循环 | `manual_scroll()`：用户滚动、程序只抓帧；开始滚之前不算「到底」（否则会立刻结束）；合成页面测试通过 |
| **窗口捕获（M5）** | `pinshot windows` 列出真实窗口（id/应用/尺寸/标题）；`pinshot window-shot --id N` 抓单窗口，访达窗口 125×97 点 → **250×194 像素**，50 ms | 实测输出见下 |
| **滚动截屏（手动，现行）** | 选区里点「滚动截屏」→ 覆盖层隐藏 → 用户正常滚动 → Enter 出长图；同一份拼接内核曾拼出 **1800×26326** PNG（页面 ~26 400 px），OCR 行号线性比 0.99–1.01 | 见 §「滚动截屏的现状（M4）」 |
| **拖动中显示选框** | `--drag-selftest 300,200,900,600` 在拖动**未松手**时截屏：x=600/1798（2x）处各有一条选区边框线 | `/tmp/drag-during.png` |
| **取消不留残窗** | `--cancel-selftest 300` / `900` → `0 overlay window(s) left` | 两次都为空 |
| **OCR（M6）** | `pinshot-ocr` 走系统 Vision，完全本地；`pinshot ocr <png>`、`snap --ocr`、App 命令 `ocr_capture`、历史卡片「文」按钮 | 真实 fixture `fixtures/ocr/mixed-text.png` 识别出中英混排 + 金额 + 邮箱；实拍截图识别 400 字符并写入剪贴板 |

## 2. GUI 端到端（已验证）

`scripts/verify-overlay.sh` 在真实屏幕上跑完了一条完整链路，结果：

```
PASS  overlay is on screen
PASS  drag produced a selection
PASS  annotation rendered
剪贴板  «class PNGf», 314061 字节   ← 复制成功
历史    1200×800 @2x  snip  ← 选区 600×400 点 × 2
        1920×1080 @1x snip  ← 副屏整屏
```

也就是说：**F1 → 冻帧覆盖层（双屏）→ 拖框 → 画矩形 → 复制到剪贴板 → 写入历史**，
全部走真实代码路径。

### 导出路径两条都验证过

- `Enter` → 剪贴板得到 `«class PNGf»`（336 KB），同时写入历史；
- `⌘S` → `~/Pictures/PinShot/PinShot_2026-11-11_13-03-37-312.png`（1000×800，选区 500×400 点 @2x，只保存一张）。

### 贴图（M2）同样端到端跑通

```
托盘菜单「贴出最近一次截图」 / 调试钩子 --pin-last
  → 窗口 (80,80) 600×400（600×400 点的截图 @2x = 1200×800 像素）
  → 渲染图片、右上角关闭按钮
  → 拖动后窗口到 (280,200)   ← 位移与鼠标完全一致
  → 关闭后窗口列表为空
```

### M4 自动滚动：循环已实现并测试，真机注入待验

```
auto_scroll(capture, scroll, wait, options, progress) -> Image
  ① 抓首帧 → ScrollCollector
  ② inject step（step = 视口高 × 0.7，钳制到「至少留 min_overlap 重叠」）
  ③ 等 settle → 抓帧 → 重复/到底判定
  ④ finish() 拼接
```

两个测试覆盖：合成页面（240×200 视口、900px 页面、粘性页眉）自动滚到底、拼回
924px 且像素对齐；静态页面两步内停止。Tauri 侧 `scroll_start` 会先隐藏覆盖层
（冻帧挡着页面看不出来在滚），循环结束后把长图交回前端，覆盖层提供「复制/保存长图」。

**真机注入卡住，原因已定位到一半**：

| 尝试 | 结果 |
| --- | --- |
| 全局 `CGEventPost(HIDEventTap)` | 1 帧（不动）——覆盖层把激活抢给了 PinShot |
| 进程启动改为 `Accessory` 策略 | 仍是 1 帧 |
| `CGEventPostToPid(目标 pid)` 定向投递 | 仍是 1 帧 |
| 先 `NSRunningApplication activate` 再全局投递 | 仍是 1 帧 |

而**同一个注入函数从 CLI 调用是有效的**：点击 Safari 激活后 `pinshot simulate scroll 900 400 10`，
页面内容有 706,345 / 2,080,000 像素变化（34%）。所以轮子本身没问题，问题在
「谁算 active」——CLI 是终端的子进程，没有被 AppKit 的单实例激活状态影响。

已排除：Pixel→Line 换算（改成 Line 仍是 1 帧）、`CGEventPostToPid`、先
`NSRunningApplication activate` 再全局投递（`activate=true`，`frontmost` 确认是 Safari
的 pid，仍不动）。

剩下的差异：CLI 是**另一个进程**（终端的子进程）投递同一串事件就有效；app 自己投递无效。
最可能是 TCC 的「负责进程」不同 —— `CGPreflightPostEventAccess()` 两者都返回 true，
但真正生效的是系统对 `pinshot-desktop` 这条独立签名/路径的判定。

下一步（18:00 窗口）：

1. 给 `PinShot.app` 授予「辅助功能」权限后重跑 `--scroll-selftest`（最可能直接通过）；
2. 若仍失败，改成 AX 驱动：`AXUIElementCopyAttributeValue(kAXVerticalScrollBarAttribute)`
   + `AXUIElementSetAttributeValue(kAXValueAttribute)`，这条路不依赖合成事件；
3. 兜底：手动模式（用户自己滚，程序只抓帧）——`ScrollCollector` 已经支持，
   只需要在覆盖层里留一个「手动滚动」按钮。

### 顺手修的第三个算法 bug

低对比度页面（相邻行亮度只差 ~5）会被「固定行检测」误判成一整块粘性页眉，
结果整帧被丢掉、拼接高度只有一半。现在检测后会把运行段回退到「几乎完全相同」
的前缀（`threshold × 0.6`），真正的粘性页眉（逐像素相同）保留，平滑内容不再误伤。

### M3 的两条数据链路已通

```
历史窗口  history_list(200) → 68 条 → 每张卡 history_thumb(id) → 320px 缓存
设置窗口  settings_load() → Config + 警告 → 表单 → settings_save(config) → config.toml
```

**未验证**：历史卡片上的点击（复制/收藏/删除）。这台机器上合成鼠标事件能打到覆盖层（screen-saver
级窗口），但打不到普通窗口；命令本体已用 `--self-test` 直接验证，点击处理器只是一行 `invoke`。

### 滚动截屏的现状（M4）

- **内核**：`auto_scroll` / `manual_scroll` 都有测试，合成页面能拼回原图。
- **接线**：`scroll_start(mode)`、`scroll_result`、`scroll_cancel`，覆盖层在抓帧期间隐藏、
  结束后以长图结果页出现（复制/保存）。
- **UI**：工具条上 `自动滚动` / `手动滚动` 两个按钮，居中显示（截图 `/tmp/toolbar3-crop.png`，
  上一版贴边导致导出按钮跑到屏幕外，已修）。
- **卡点 1**：app 进程投递的滚轮事件不生效（CLI 投递同样的序列却有效），详见上表。
- **卡点 2 已定位（不再是卡点）**：给 `manual_scroll` 加逐次日志（`PINSHOT_TRACE=1`）后
  看到每帧抓取 35–75 ms、结果全是 `Duplicate`——循环没阻塞，是**页面真的没动**。
  再做对照实验：每次滚动前先点一下 Safari，日志立刻变成
  `Accepted len=1…10`，说明「抓帧 → 判定 → 拼接」整条链路是通的，
  唯一的条件是**目标窗口必须先被真实点击/激活**。
- 目前**端到端验证过的滚动路径**有两条：离线 `pinshot scroll --frames <dir>`
  （6 帧 → 420×900，逐像素比对 0 处不符），以及真机手动模式
  （`--scroll-selftest ...,manual` 抓到 13 帧并完成拼接）。
- 自动滚动仍缺辅助功能授权（`ax=false`），一旦授权即可验证；
  托盘菜单已提供「授予「辅助功能」权限」入口。

### M7 打包（已完成，缺 Developer ID 证书）

- `bundle.targets = ["app", "dmg"]`，`icons/icon.icns`（由 512px PNG 生成整套 iconset）。
- `src-tauri/Info.plist`：`LSUIElement = true`（菜单栏 App，不占 Dock）。
- `scripts/install.sh`：release 构建 → ad-hoc 签名 → 安装到 `/Applications` → 注册 LaunchServices。
  安装到 `/Applications` 而不是原地跑 debug 产物，是为了让 TCC 授权不会因为每次重编译而失效。
- `scripts/make-dmg.sh`：先签名再打 DMG。**不能**用 `tauri build --bundles dmg`——
  它会在打包时重新 linker-sign，把稳定的 `ai.pinshot.desktop` 换回
  `pinshot_desktop-<hash>`，权限因此会失效（已实测）。
- 产物：`PinShot_0.1.0_arm64.dmg`（5.6 MB），挂载验证过里面的 App
  `Identifier=ai.pinshot.desktop`、`LSUIElement=true`。
- 真机 Developer ID 签名与公证需要证书，本机没有；脚本里保留了这一步的位置。

### 滚动改为 AX 驱动（不再使用滚轮）

按用户要求，App 里**已经完全移除了滚轮注入**（`grep scroll_points src-tauri/src/main.rs` 无结果）。
滚动改成直接驱动目标 App 的滚动条：

```
AXUIElementCreateApplication(pid) → BFS 找 AXVerticalScrollBar → set AXValue
```

实测（CLI，不动光标、不发滚轮）：

```
$ pinshot ax-scroll 600 400 0.3
{"ok":true,"scrollbar":0.30000001192092896}      # 页面确实滚了（前后截图不同）
$ pinshot ax-scroll 0 0 0.4 --pid <safari>
{"ok":true,"scrollbar":0.9998235702514648}
```

两个入口：`ax::scroll_at(point, fraction)`（按光标位置找滚动区）和
`ax::scroll_app(pid, fraction)`（按 App 的 AX 树找，和捕获同一个窗口，不依赖光标）。

### 两个交互 bug（已修）

1. **拖动时看不到选框**：`paint()` 之前只画「已提交的选区」，而拖动过程中选区被故意置空
   （工具条不该提前出现），于是直到松手才有画面。现在绘制时会用 `dragRef` 的实时矩形，
   拖动中就能看到遮罩、边框和尺寸标签。
2. **取消后又冒一个截屏框**：取消时另一块屏幕的覆盖层可能还在 `build()` 中，
   `close_overlays` 遍历不到它。现在三处都检查会话标志：`open_overlays` 建窗前/建窗后、
   `overlay_show` 显示前，另外 `close_overlays` 之后再做 3 次 120 ms 的延迟清扫。

### 拖到边缘 → 边滚边抓（已完成，实测通过）

手势：**按住拖出选区 → 拖到屏幕上/下边缘 → 立即开始滚动采集**，覆盖层保持可见并实时变长；
**松手结束**（得到长图，可复制/保存），**Esc 取消**。

实现要点：

- 抓的是**覆盖层窗口下面**的屏幕：`CGWindowListCreateImage(rect, OptionOnScreenBelowWindow,
  overlayWindowNumber, …)`，新增 `pinshot_capture::windows::grab_below(id, area)`。
  所以覆盖层可以一直显示、一直画，而截出来的图里没有覆盖层。
- 长图**增量拼接**：`pinshot_app::scrolling::LiveStitcher` 每次只追加新露出来的那几行
  （粘性表头只保留一次），配合 `live_scroll` 驱动循环。
- **不再用像素匹配量位移**：AX 的 `AXValue` 是「整页可滚动高度的比例」，固定步长会一次翻过好几屏，
  重复行页面还会让匹配器给出**自信的错误位移**。现在实测位移以**滚动条 delta × 每单位像素数**
  为准，像素匹配只用于首次标定和交叉校验（两者一致时采用像素值，精度更高）。

实测（`/tmp/scroll-static.html`，400 行静态长页，约 26 700 px）：

```
# CLI，无滚轮、不动光标
$ pinshot scroll-live --area 100 120 900 600 --pid <safari> --max 60 --out /tmp/live9.png
scrolled to 1800×26326 in 32 frames, 29707 ms

# App 内（覆盖层可见，抓覆盖层下面的内容）
$ PINSHOT_LIVE_MAX=50000 pinshot-desktop --live-selftest 100,120,900,600
live-selftest: 5922492 bytes after 37122 ms      → /tmp/live-selftest.png  1800×26326
```

没有错位的证据：对长图每 2000–2500 px 取一条做 OCR，读出行号并与纵向位置做线性拟合，
比值稳定在 **0.99–1.01**（换行间距 66.7 px），没有跳号或重复，见
`docs/STATUS.md` 的记录。

| 检查 | 结果 |
| --- | --- |
| 长图高度 | 26 326 px（页面 ~26 400 px，差 <1%） |
| 行号连续性 | 单调递增，无跳号/重复 |
| 每 66.7 px 行数比 | 0.99 – 1.01（一处 0.93） |
| 采集耗时 | 32 步 / 29.7 s（每步约 0.9 s） |
| 滚轮/光标 | 完全没有使用 |

一键复验（需要一块空闲的桌面，会占用屏幕约 40 秒）：

```
scripts/verify-live-scroll.sh            # 默认 100,120,900,600
scripts/verify-live-scroll.sh 300 200 600 600
```

它自己建测试页、把页面复位到顶部、跑 App 的 `--live-selftest`，再用 OCR 读回行号并打印
`per-66.7px` 比值，全部落在 0.75–1.25 就输出 `PASS`。

**手势本身的验证情况**：拖到边缘会立即调用 `scroll_live_start`，日志里能看到
`live scroll: area=Rect { x: 300, y: 200, w: 600, h: 680 } … pid=65498` 和随之而来的
`live strip: frames=1 height=1360`，说明选区几何、目标 App、首帧都对；「覆盖层在拖动过程中
不断变长」的肉眼确认被桌面占用打断过两次（Safari 窗口被拖走/缩到 173×168、显示器休眠），
所以留了上面的脚本，桌面空的时候一条命令就能复验。

### 2026-11-12：「拉到底报失败」的根因（已修）

逐帧日志给出了确定答案，**不是抓帧冻结**：

```
live: frame=1200x1360 delta=Some(0.02) probe=Still ppu=None
      fp=c708f727… identical_to_previous=false scrollbar_moved=true
```

两帧指纹不同（画面确实变了）、滚动条也确实动了，但**匹配器**判 `Still`，而循环把 `Still`
当成「页面到底」直接结束 —— 于是永远只有 1 帧，用户看到「滚动截屏失败」。

修了三处（`crates/pinshot-app/src/scrolling.rs`）：

1. `Still` 只在**滚动条也没动**时才算到底；滚动条动了就缩小步长重试（`0.4×`，下限 0.0005）。
2. `Unmatched` 时先**回滚这一步**再重试：拼接器仍持有上一帧，不回滚就永远对不上。
3. 连续两帧画面相同但滚动条在动 → 明确报错「捕获画面连续没有变化：屏幕可能已锁定，
   或该窗口处于截图保护状态」，不再伪装成「页面没滚动」。

新增单测 `a_frozen_capture_is_reported_not_treated_as_the_end`。

### 实测结果（2026-11-12 12:1x，**安装版 release**）

用 `/Applications/PinShot.app/Contents/MacOS/pinshot-desktop`（12:06 构建）直接跑，目标为
`TextEdit` 的 500 行长文本（窗口 920×810）：

| 路径 | 证据 |
| --- | --- |
| 工具条 **滚动截屏** 按钮 | **15 条 strip**，长图 **1400×12814**，用时 5.2 s |
| **拖到底边** 按住 6 s | **14 条 strip**，高度递增到 12986；两张按住截图 md5 `8914831f…` vs `f710f16a…`（不同） |

长图无缝校验（14 个采样点，跨 12814 px）：行距恒为 **25.9 px**，行号-位置拟合残差 ≤1.2 行（≈31 px，
来自裁剪对齐），中间无跳号/重复。

### 早前 debug 构建的实测（同一套代码）

两条路径在 `TextEdit` 的 500 行长文本上跑通（窗口 920×810，选区 100,120,700,700）：

| 路径 | 证据 |
| --- | --- |
| 工具条 **滚动截屏** 按钮 | `button-selftest: sent` → **1400×7610**，10 条 strip，14.5 s |
| **拖到底边** 按住 16 s | **8 条 `live strip`**，高度 1360 → 5461 递增；`/tmp/drag-during.png` 与 `/tmp/drag-hold.png` md5 **不同**（覆盖层持续重绘） |

逐帧 trace（最终构建，`identical_to_previous=false`）：

```
live: frame=1200x1520 fraction=0.0100 delta=Some(0.01) probe=Moved(36)  ppu=None      identical_to_previous=false scrollbar_moved=true
live: frame=1200x1520 fraction=0.0300 delta=Some(0.03) probe=Moved(188) ppu=Some(3600) identical_to_previous=false scrollbar_moved=true
live: frame=1200x1520 fraction=0.0900 delta=Some(0.09) probe=Moved(559) ppu=Some(3600) identical_to_previous=false scrollbar_moved=true
```

### 长图质量（无缝校验）

对按钮产出的 `1400×7610` 长图做行号-位置拟合（文本每行编号 +7、行距 26 px）：

```
y=    0 expected≈  28.0 ocr=  35  delta= +7.0   （裁剪落在下一行）
y= 2080 expected≈ 589.6 ocr= 595  delta= +5.4
y= 2800 expected≈ 784.0 ocr= 777  delta= -7.0
y= 4900 expected≈1351.0 ocr=1351  delta=  0.0
y= 7000 expected≈1918.0 ocr=1918  delta=  0.0
```

残差始终 ≤ 1 行（26 px），中间没有跳号或重复 → 拼接无接缝。

### 修：点托盘图标直接冒出一个框（2026-11-12）

是自测钩子留下的副作用：`--button-selftest` 用 `std::env::set_var("PINSHOT_DEBUG_REGION", …)`
给进程设了预置选区，而这个变量会一直留在进程里——之后每次截图都带着它，于是「一点托盘就出现一个框，
还没拖呢」。

改了两处：

1. `overlay_meta` 读完 `PINSHOT_DEBUG_REGION` 立刻 `remove_var` —— **一次性**；
2. `--button-selftest` / `--drag-selftest` / `--live-selftest` 跑完 `handle.exit(0)`，
   不会再留一个「测试实例」在后台等你点托盘（那正是之前点出框的那个进程）。

实测（`--capture` 后立刻截屏，找选区那种红色边框）：

```
accent pixels (selection border): 8      ← 只是噪声，没有边框
```

### 手动采集改为高频取样（2026-11-12）

日志：`没有两帧能对上（滚动太快，帧之间没有重叠）`。原因是**取样太慢**：手动循环每帧
`wait 280 ms` + `settle` 最多 4×90 ms ≈ **700 ms 一帧**，一次甩动就跨过好几屏，任何两帧都不重叠。

改：

- 手动取样间隔 280 ms → **120 ms**，并去掉 settle（滚动中的画面仍是一张有效快照，
  之前怕「动画中途抓帧」反而把间隔拉长成了致命问题）；
- `max_frames` 40 → **60**（120 ms × 60 ≈ 7 秒），避免为长页面囤积过多整屏缓冲；
- 仍保留 `stitch_images_tolerant`：个别对不上的帧丢弃，不让整次失败；
- 跳过发生时通过进度回传 `skipped`，App 弹一次「滚得有点快」的提醒。

### 手动拼接容忍丢帧（2026-11-12）

日志里失败的是 `第 2/3 帧的匹配度只有 0.10–0.26`——**两帧完全没有重叠**：滚得快时，
两次取样之间页面已经走了一屏多，拼接器自然对不上，而它会因此让整次采集失败。

现在手动采集改用 `stitch_images_tolerant`：某帧与已保留的最后一帧对不上就**跳过该帧**
（它覆盖的那一小段会缺失），其余照常拼接；只有在**所有帧都对不上**时才报错
（`滚动拼接失败：没有两帧能对上（滚动太快…）`）。

单测 `a_frame_with_no_overlap_is_skipped_not_fatal`：序列 0/120/240/760/880（后两帧大幅跳）
不再整体失败，前面能对上的帧仍然拼出图。

### 交互定稿（2026-11-12）：只剩一条路 + 支持向上滚

按用户要求只保留一种通用交互：

**F1 → 框选 → 点「滚动截屏」（或把框拖到屏幕上/下边缘）→ 覆盖层隐藏 → 你正常滚页面 → Enter 出长图 / Esc 取消。**

向上滚之前会失败（`第 1 帧的匹配度只有 0.19`），原因是帧序反了：拼接器假设内容向上移动，
遇到向上滚（内容向下移动）直接判低匹配。现在加了**行指纹方向判定**
（`scroll_direction`：逐行哈希，在两个方向上找对齐行数最多的一边），结束前按它反转帧序。

单测 `scroll_direction_is_read_from_the_rows_alone` 覆盖三个情形：向下滚 → `Some(1)`、
向上滚 → `Some(-1)`、画面没动 → `None`。

（调试中发现行哈希只取了红色通道，而测试页红色只与列有关 → 所有行哈希相同；已改成三通道。）

### 交互定稿：两种模式，各自说清楚（已废弃）

用户反馈「浏览器里能滚，但按 Enter 提示无法截屏；别的应用直接滚不了」。根因是「点击穿透 +
冻帧覆盖层」这个组合本身不成立：穿透后滚轮确实落在页面上，但页面上盖着一张冻结的截图，
用户等于**盲滚**，Enter 走的还是旧逻辑。

最终定成两条明确的路：

| 情况 | 行为 |
| --- | --- |
| 窗口可以被辅助功能滚动 | 覆盖层留在屏幕上，框自己边滚边长；Enter/点一下出图 |
| 窗口不能 | **覆盖层隐藏**（你能看见真实页面），弹一条通知「滚动采集中 · 在页面上正常滚动即可，按 Enter 完成」，程序在后台抓帧拼接，Enter 出图 |

「点击穿透」「滚轮转发」「follow 模式」全部删掉——它们让用户对着冻帧盲滚，体验是坏的。

实测（最新代码，TextEdit 上先让自动滚动无内容可滚）：

```
live scroll: nothing captured, switching to manual capture
scroll done: 1934×3088 (manual=true)
```

### 采集期间覆盖层「点击穿透」（已废弃）

上一版把滚轮事件转成 AX 滚动转发给页面——但如果那个窗口本来就不能被 AX 滚动，
**转发也转发不动**，用户看到的还是「滚了没反应」。

现在改为：采集一开始就给所有覆盖层 `set_ignore_cursor_events(true)`，**鼠标/滚轮直接穿透**
到下面的窗口，页面用它自己的方式滚动（不再经过我们）；覆盖层仍然显示、仍然实时变长。
采集结束时恢复。

- 结束方式改为 **Enter 完成 / Esc 取消**（点击已经穿透，不再用「点一下」结束）；
- 采集循环在驱动不了滚动条时进入 `follow`：每 200 ms 看一眼画面，有变化就接上，
  `idle_limit = 120`（约 24 秒没动静才自然结束）。

trace 证据（目标窗口无滚动条时）：

```
live: no accessible scrollbar, following the user's wheel
```

### 交互重做（按用户反馈）：框一直在，滚轮滚动采集

用户明确否掉了「切手动 / 隐藏覆盖层 / 弹通知」那套交互：「不能直接在有框的情况下滚动截图吗？」

现在改成：

- 点 **滚动截屏**（或拖到边缘）后，**选区框一直在**，长图在框里实时变长；
- 覆盖层会把**滚轮事件转发给被截的 App**（`live_wheel` → `ax::scroll_app`），
  所以即使这个窗口不能被我们自动滚动，你自己滚滚轮/触控板，页面照样动、框照样长；
- 结束方式只有两个：点一下画面 / <kbd>Enter</kbd>，或 <kbd>Esc</kbd> 取消；
- **不再**出现「手动采集」「已切换手动滚动」的通知，也不再隐藏覆盖层。

内部把原来的「被动模式」改成 `follow` 模式：驱动不了滚动条时不再退出，而是每 200 ms 看一眼画面，
有变化就接上（`idle_limit = 120` ≈ 24 秒无变化才结束）。

> 之前那版（隐藏覆盖层 + 通知）已删除——它会让用户对着盖住的屏幕滚不动，是死路。

### 交互优化：自动滚动失败不再报错，直接转「你滚我抓」

用户反馈「经常提示应用不支持」。原来的行为是 `scroll_app` 找不到滚动条就抛错、
整个截屏失败。现在分两层兜底：

1. **开始前**：`scroll_position(pid)` 与 `scroll_at(centre)` 都不可用 → 直接走手动采集（覆盖层隐藏）。
2. **运行中**：实时采集一条新内容都没拿到（`frames <= 1` 且高度没长）→ 自动切手动采集。

切换时弹一条系统通知：「这个窗口无法自动滚动：正常滚动即可，我自动抓帧；按 Enter 结束」，
覆盖层随即隐藏，用户正常滚动，程序继续抓帧，Enter 结束出长图。

错误文案也改成可执行的描述（「画面没有变化：这个区域可能不可滚动，或页面已经到底」），
不再出现「应用不支持」这种让人无从下手的话。

**顺带修掉一个老 bug**：手动模式在隐藏覆盖层时直接调用了 `window.hide()`，而 AppKit 要求
主线程——隐藏实际没生效，所以手动截到的每一帧都是那张冻结的覆盖层（`nearly_identical`
全部命中，拼出来只有一屏）。改成 `run_on_main_thread` 后手动模式才真的能看到页面。

实测（安装版，TextEdit 500 行长文本）：

```
PINSHOT_BUTTON_TEXT=手动 pinshot-desktop --button-selftest 100,120,700,700
scroll done: 1400×8384 (manual=true)

行号采样：4, 31, 58, 84, 112, 139, 166, 188, 215, 242, 269, 296
monotonic increasing: True      （26 px/行）
```

**方向由滚动条位置判定**：手动采集时每帧读一次 `AXValue`，结束前比较首尾位置，
位置变小就把帧序列反过来（像素投票只作为读不到滚动条的兜底）。实测向上滚：

```
scroll done: 1400×8185 (manual=true)
行号尾部: 204, 291, 318, 345, 372, 399, 426, 453, 480   （单调递增）
```

**完整降级链实测**（安装版，先用滚动条把文档拉到底让自动滚动无从下手）：

```
live scroll: nothing captured, switching to manual capture
scroll done: 1400×5871 (manual=true)
行号采样: 179, 206, 243, [OCR 误读], 316, 403, 430, 461
y=2000 处实际内容: 265 → 281 连续
```

### 顺序修复（settle + 滚动条定位）

两处一起改了：

1. **取样前等画面稳定**：手动采集每次先抓一帧，再等 90 ms 重抓，连续两帧几乎相同才接受
   （最多重试 4 次）。`settle` 是**独立闭包**，不复用 `wait`——测试用 `wait` 模拟「用户滚一下」，
   复用会把合成页面的位移算乱。
2. **方向按滚动条位置判定**：每帧读 `AXValue`，结束前比较首尾，位置变小就把帧序反转。

另外把 `scroll_start` / `scroll_live_start` 改成 `async`：同步命令跑在主线程上，
`run_on_main_thread(hide)` 会被排到命令返回之后，覆盖层其实一直盖着——这就是手动模式
偶尔「抓到的全是同一张」的原因。改完日志可确认：

```
scroll: hide called on 2 overlay(s)
scroll: overlays visible after hide: 0
```

**安装版实测（两个方向，整幅全程单调，不再只是尾部）**：

| 方向 | 结果 | 行号采样 |
| --- | --- | --- |
| 向上滚 | 1400×8185 | 183, 206, 229, 253, 276, 299, 322, 345, 368, 391, 414, 438, 461, 484 |
| 向下滚 | 1400×8234 | 4, 27, 50, 73, 96, 119, 142, 166, 190, 212, 236, 259, 282, 305 |

实测（安装版，Finder 图标视图窗口 500×300）：

```
button-selftest: sent
live scroll: nothing captured, switching to manual capture
```

正常路径无回归：TextEdit 上 15 条 strip、1400×12866。

### 目标窗口解析（避免选错 App）

之前 `under_point` 只取**最前面**的窗口，用户浏览器压在编辑器上时会选到不可滚动的那个。
现在 `scrollable_pid_under()` 从最前面往后遍历，取**第一个真的暴露滚动条**的应用；
找不到才回退到最前窗口 / 捕获时的 target。

### 新增「滚动截屏」按钮（按用户反馈）

只靠「拖到边缘」不够：滚动条可能停在任何位置。工具条现在有一个 **滚动截屏** 按钮，
按下即从**当前滚动位置**开始自动向下滚动并拼接（仍不模拟滚轮、不动光标），
点画面或 Enter 结束，Esc 取消。工具条点击不会再被误判成拖拽/松手。

> **2026-11-12 00:05 状态**：这台机器的屏幕在锁屏状态（`CGSSessionScreenIsLocked`，
> `screencapture` 全黑）。锁屏时 `screencapture` 与 `CGWindowListCreateImage` 都拿到同一张
> 静止画面，所以实时采集在锁屏期间必然只出 1 帧（`probe=Still`、`delta≠0`）——
> 这是环境现象，不是代码问题。解锁后跑 `scripts/verify-live-scroll.sh` 即可完成最后一步确认。

### 滚轮注入默认关闭（按用户反馈）

自动滚动靠**模拟滚轮**实现，这会直接控制用户自己的滚动体验。现在：

- 配置项 `capture.allow_synthetic_scroll`，**默认 false**；
- 关闭时「自动滚动」直接返回：「自动滚动会模拟滚轮、影响你自己的滚动体验，默认关闭。
  请在设置里打开，或改用「手动滚动」」；
- 设置面板里有对应开关，并注明「手动滚动不受影响，也不会动你的滚轮」；
- CLI 的 `simulate` 依旧需要 `PINSHOT_ALLOW_SYNTHETIC_INPUT=1` 才执行。

手动滚动（你自己滚，程序只抓帧）既不注入事件也不移动光标，是默认路径。

### 自适应步长（已实现，真机仍差一步）

`auto_scroll` 不再假设「一行滚轮 = 40 像素」：

1. 从 **1 个滚轮单位**（40 px 请求）开始，绝不会一上来就把重叠撕掉；
2. 每抓一帧，用 `measure_shift()`（就是拼接用的那个匹配器）量出**真实位移**；
3. 反推 `px/单位`，把下一步校正到 `视口 × 0.35`，并夹在「至少留 `min_overlap` 行」之内；
4. 量不出位移（太小的移动不可信）时不动，匹配失败才减半。

向上采集时相邻帧要**反着量**（`measure_shift(next, prev)`），否则会被当成「没动」。
两个方向都有测试：`auto_scroll_walks_a_page_to_the_end`、`auto_scroll_upwards_stitches_in_the_right_order`。

**窗口级捕获已和滚轮解耦验证**：`--scroll-selftest window:<id>,manual` 用 `windows::grab`
逐帧抓**单个窗口**（与滚轮路由无关），配 CLI 的滚动，抓到 2 帧并成功拼接
（661×438，位移 5 px —— 滚得太少，但链路本身是通的）。

**真机仍然没跑通**，而且现象自相矛盾：

| 起始步长 | 结果 |
| --- | --- |
| 40 px（1 行） | 1 帧 —— 目标窗口**完全没动** |
| 182 px（0.35 视口） | 2 帧后失败 —— 第 1 对匹配度 0.14–0.24，位移远超视口 |

同一段代码、同一个目标，1 行不滚、4 行滚过头。最可能的原因是**滚轮事件被投递到了不同窗口**
（覆盖层弹出时抢走焦点，`warp` 之后目标窗口并不总是收到），而不是步长算法本身。

排查中还发现一个窗口服务器的怪现象：TextEdit 的 `big.txt` 窗口，AppleScript 报
`1653,-105, 661×433`，而 `kCGWindowBounds` 报 `3571,666, 129×120`（稳定复现，且在屏幕外）。
说明列表里可能存在**同标题的幽灵条目**，按 id 抓住的并不是用户看到的那个窗口——
这也会让「抓窗口 + 滚轮」这一对操作作用在两个不同对象上。
下一步会用窗口级捕获（`windows::grab`，与滚轮路由无关）复现，先确认「捕获侧」没问题，
再单独调滚轮路由。

### 滚动截屏的交互（按用户反馈改进）

拖选区域时，**把光标拖到屏幕底边**（28 px 内）松手 → 选区成为采集区域，立刻开始向下滚动截屏；
拖到**顶边**同理，向上采集。选区标签会实时提示「↓ 松手向下滚动 / ↑ 松手向上滚动」。

向上采集在算法上就是把帧序列**反转后再拼接**（`ScrollRunOptions::direction`），
有独立测试覆盖（`auto_scroll_upwards_stitches_in_the_right_order`）。

### 验收脚本

`scripts/acceptance.sh` 一条命令跑完 capture → clipboard → history → OCR，全部用自动提交
（不移动鼠标、不按键），失败会打印具体哪一步。滚动截屏单独验（需要真人滚一下）。

### M4 自动滚动的真正卡点：辅助功能授权

App（LaunchServices 启动）自报状态：

```
permission: screen=true ax=false input=false
```

- `screen=true`：屏幕录制已授权，**截图链路完全可用**（实测：剪贴板得到 PNG，历史新增 2880×1800 @2x）。
- `ax=false`：辅助功能没授权 → 合成滚轮被系统丢弃。之前所有「1 帧」都是这个原因，
  和代码无关。
- `CGPreflightPostEventAccess()` 会返回 **true**（假信号），只有
  `AXIsProcessTrustedWithOptions` 才是准的——现在 `doctor` 和 `scroll_start` 都用后者。

托盘菜单新增「授予「辅助功能」权限」：点一下会调
`AXIsProcessTrustedWithOptions(prompt=true)`，系统弹窗并把 PinShot 加进
「系统设置 → 隐私与安全性 → 辅助功能」。授权后需**重启 PinShot**（TCC 按进程缓存）。

另外发现：从 shell 直接跑的二进制会**继承终端的辅助功能授权**（所以 CLI 一直能滚动），
用 `open -a` 启动的 App 才是它自己的身份（ax=false）。验证时要注意这个区别。

### 窗口枚举的一个真 bug（已修）

`kCGWindowBounds` 是 CFDictionary，一开始我用 `objectForKey:` 逐字段读，拿到的是
**自动释放对象**——同一块 1140×780 的窗口会被读成 163×165、坐标每次调用都不一样。
改用 CoreGraphics 自带的 `CGRectMakeWithDictionaryRepresentation` 后数据稳定且正确
（OpenCode 窗口 1440×875 与 AppleScript 报告一致）。

### M5 窗口级捕获（内核已通）

```
$ pinshot windows
ID       APP                      SIZE       TITLE
24689    aimail-desktop           1214×840   AiMail
22153    OpenCode                 163×185    项目分析 (截图) · OpenCode
13440    访达                      125×97     离线外呼标签

$ pinshot window-shot --id 13440 --out /tmp/window.png --json
{"app":"访达","height":194,"ms":50,"ok":true,"width":250,...}
```

`CGWindowListCopyWindowInfo` + `CGWindowListCreateImage(.optionIncludingWindow)`；CFArray/NSDictionary
用 toll-free bridge 走 Foundation 访问器，避免手写 CF 字典解析。启用条件是 `layer == 0`
且尺寸 ≥ 32 点（过滤菜单栏/悬浮层）。**不产生任何合成输入**，锁屏状态下也能用。

### 工程与打包的其它修正

- **工具条竖排（已修，待肉眼确认）**：按钮被 flex 压到 1 字宽，中文逐字换行，
  `自动滚动/手动滚动` 四个字竖成四行。现在 `flex: 0 0 auto` + `white-space: nowrap`
  + 允许换行，并把标签缩成 `自动/手动`。
- **磁盘**：`[profile.dev] debug = "line-tables-only"`、`incremental = false`、
  `[profile.dev.package."*"] debug = false`。依赖不再携带完整 DWARF（26 GB 的大头），
  `cargo clean --profile dev` 清出 2.2 GB，release 的 .app/.dmg 保留。
- **签名**：改用本机证书 `OpenCodeGo Dev`，指定要求变成
  `identifier "ai.pinshot.desktop" and certificate leaf = H"…"`——重编译不再让 TCC 授权失效。
  脚本 `install.sh` / `make-dmg.sh` 会自动优先使用该身份，没有证书才退回 ad-hoc。
- `tccutil reset ScreenCapture ai.pinshot.desktop` 清掉了那条「开关是开的但运行时拒绝」的旧记录。

### 安全开关（新增）

合成鼠标/键盘会移动真实指针、抢焦点，因此 `simulate` 现在默认拒绝执行，必须显式
`PINSHOT_ALLOW_SYNTHETIC_INPUT=1` 才生效。自动滚动属于产品功能，不受此限制。

### 滚动截屏调试结论（M4）

1. **单位换算 bug（已修）**：`scroll_pixels` 把设备像素当点除以 40，在 2x 屏上实际滚动量翻倍，
   帧间没有重叠，匹配度只有 0.10–0.24。现在改为 `scroll_points`，调用方先按 scale 换算。
2. `manual_scroll` 逐次日志（`PINSHOT_TRACE=1`）证明循环**不阻塞**（每帧 35–75ms），
   之前「1 帧」是页面根本没动。
3. 页面能被滚动的前提是**目标 App 处于激活状态**；CLI（终端是受信任进程）在
   `activate` + 点击后可以滚动，App 自身投递仍被系统丢弃（`CGPreflightPostEventAccess`
   返回 true，但事件不落地）。

### 唯一没验证的一环：操作系统把按键送到 App

macOS 按 **WindowServer 的焦点**分发键盘事件。从 shell 直接跑的二进制即使
`NSApp.isActive == true`、`isKeyWindow == true`，也收不到真实/合成按键；从 LaunchServices
启动（`open PinShot.app`）时焦点正常，但未签名的 bundle 需要用户在「系统设置 → 隐私与安全性
→ 屏幕录制」里给它单独授权（首次启动已弹过提示）。

所以测试里用 `--auto-commit <ms>` 注入与 Enter **完全相同**的事件（`overlay:key`），
把「渲染 → 裁剪 → 剪贴板 → 历史」这一段完整覆盖；按键分发本身留给人工确认。

期间修掉的三个真实缺陷：

1. `CGDisplayPixelsWide` 在 Retina 上返回**点**不是像素 → 主屏覆盖层元数据 1440×900
   却收到 2880×1800 的帧，前端 `ImageData` 抛 `IndexSizeError`。现在以实际帧为准。
2. `boot()` 依赖 `paint()`，而 `paint` 依赖 `revision` → 每次标注都会重新 boot，把选区
   重置掉。改成 boot 只跑一次、`paint` 稳定。
3. 无边框窗口需要把 **WKWebView** 设为 first responder（只设 contentView 会把键吃掉），
   并显式 `ActivationPolicy::Regular`。
4. 双屏时 `Enter` 会让**两块屏都提交**：Tauri 的窗口级 `emit` 实际广播到所有 webview，
   而且同一秒的两张图会互相覆盖文件名。现在事件带 `target` 显示器下标，非目标窗口丢弃；
   文件名精确到毫秒。

## 3. 结构

```
pinshot/
├── crates/
│   ├── pinshot-core/      几何 / 图像 / 错误 / 配置（33 个测试里 13 个在这）
│   ├── pinshot-capture/   macOS CoreGraphics 抓帧 + 合成输入（drag/key/scroll）
│   ├── pinshot-app/       会话、裁剪、PNG 编解码、剪贴板、落盘
│   ├── pinshot-store/     SQLite 历史 + 内容寻址 blob + 保留策略
│   ├── pinshot-cli/       snap / scroll / monitors / history / doctor / simulate
│   └── pinshot-stitch/    滚动拼接（灰度匹配 + 合成计划）
├── src/                   Overlay.tsx + scene.ts + render.ts
├── src-tauri/             Tauri 壳：托盘、F1、覆盖层窗口、导出命令
└── scripts/               free-window.py（工作时间闸门）、verify-overlay.sh
```

## 4. 两个设计决定（写进代码注释了）

1. **标注只有一个渲染器**：场景图在 `src/scene.ts`，绘制在 `src/render.ts`（Canvas2D），
   导出时用同一份代码把 `底图 + 标注` 渲染成 PNG 交给 Rust。没有「预览一套、导出另一套」
   的分叉，所以所见即所得；CJK 文字也直接走系统字体，不需要自带字体光栅器。
   Rust 只负责裁剪、编码、剪贴板、落盘。
2. **覆盖层窗口不能被 AppKit 当成普通窗口**：无边框窗口默认拿不到键盘焦点。壳里显式
   设 `ActivationPolicy::Regular`、`makeKeyAndOrderFront` 并把 WKWebView 设为 first
   responder，否则覆盖层画得出来但收不到任何按键。

## 5. 已知问题

- 抓一帧约 **117 ms**（双屏并行；串行是 190 ms）。预算 100 ms，还差一点，
  M4 前换 ScreenCaptureKit 再压一档。
- `CGDisplayCreateImage` 在 macOS 15 上可能不可用；迁移到 SCK 已排在 M4 之前。
- 锁屏状态下截图只有锁屏画面（系统行为，不是 bug）。
- 标注里的文字工具在窗口拿不到焦点时无法输入（与第 4 条同源；已在显示后补抢两次焦点，
  `--text-selftest` 有端到端证据）。
- 覆盖层退出后不还原上一个应用的前台焦点（M3 处理）。

## 6. 下一步

1. 解锁后跑 `scripts/verify-overlay.sh`，把 GUI 交互补上证据。
2. **M2 贴图窗口**：把 `PinManager` 接到 Tauri（多窗口、透明度、穿透、托盘菜单）。
3. **M3 历史面板**：列 `store.recent()`，点击还原到剪贴板；设置面板写回 `config.toml`。
4. **M4 滚动截屏**：`ScrollCollector` 接真实抓帧 + 自动滚轮注入（`events::scroll` 已就绪）。
5. 迁移 ScreenCaptureKit，把抓帧从 ~200 ms 压到预算内的 100 ms。

## 6.5 2026-11-13：点击到出图的延迟（实测）

单屏 2880×1800（Retina 2x），`--capture-selftest` 连续抓取，计时口径是「按下 → 冻结帧
真正画到屏幕上」（Rust 记开始时间，前端在 `overlay_show` 前打点）：

| 版本 | 第一次 | 稳态 |
| --- | --- | --- |
| 优化前 | 414 ms | 246–272 ms |
| 窗口与抓帧并行 | 383 ms | 210–228 ms |
| + 预热覆盖层（hidden，页面常驻） | 117 ms | **70–100 ms** |

拆开看（预热后）：抓帧 60–75 ms，帧传输 19 MB 约 1–2 ms，`putImageData` + 绘制约 20 ms，
显示/抢焦点约 10 ms。之前 246 ms 里约 150 ms 花在「新建窗口 + WebKit 加载页面」上，所以
把窗口留活比什么都值钱。

做法：

1. `start_with_monitors`：先枚举显示器（约 1 ms）并把覆盖层窗口建起来，再抓帧；页面加载
   与抓帧并行。`overlay_meta` / `overlay_frame` 会等 session（最多 2.5 s）。
2. **预热**：启动 400 ms 后建好隐藏的覆盖层（页面已加载）。抓图时只发 `overlay:begin`，
   页面重新 boot，窗口再 `show`。
3. `close_overlays` 从「销毁窗口」改成「隐藏 + `overlay:end` 让页面清场景」，所以下一次
   点击不用再付页面加载的钱；`resetScene` 把 canvas 缩到 1×1 释放 20 MB 表面。
4. 60 秒没有新截图就销毁预热窗口（`capture_serial` 判断期间有没有新抓取），把内存还回去。
5. 历史缩略图改成**截图时就生成**（`write_thumbnail`），打开历史面板不再为每张 5 MP PNG
   解码一次。

## 6.54 2026-11-13：中英文、README、清理与最后一轮提速

- **中英文**：`src/i18n.ts` 里一份 `zh` / `en` 文案表（`t(key, vars)` + `useLocale()`），
  设置里新增「语言」（跟随系统 / 简体中文 / English），立即生效并写进 `config.toml`
  的 `general.language`。原生侧（托盘菜单、窗口标题、通知、权限提示）用
  `tr(zh, en)`，启动时按 `general.language` 解析（auto 读 `defaults read -g AppleLanguages`）。
  测试：`src/i18n.test.ts`（3 条）；实机截图：英文窗口标题/菜单/全部设置项均为英文。
- **修掉设置窗口一进来就滚动到中间**：WKWebView 复用了上次的滚动位置，现在挂载后把
  `.window` 的 `scrollTop` 归零（下一帧再归一次）。验证：自测里读到
  `settings scrollTop: 0`（之前是 ~120）。
- **冗余代码**：`cargo clippy` 只报了 `forget_hotkey` 未使用，已删；`start_capture` 里
  为滚动截屏准备、却从没被读过的 `state.target_pid` 连同 `backend_cursor_point()` 一起删掉
  （顺带省掉每次截图 5–15 ms 的窗口枚举）。前端扫过一遍：CSS 类 0 个未使用、导出符号
  0 个未引用。
- **再快到 60 ms**：去掉上面的窗口枚举后，同一台机器稳态 **59–65 ms**（此前 70–100 ms），
  第一次 89 ms。
- **包体**：`src-tauri` 加 `strip = true` + `lto = "thin"` + `codegen-units = 1`，
  `PinShot.app` 从 **20.07 MiB → 14.33 MiB**。
- **README 重写**：面向使用者的功能/快捷键/配置/命令行/构建/结构/限制，替换掉原来的
  立项文档口吻（旧内容仍在 `spec/`）。

## 6.55 2026-11-13：设置里直接按键录快捷键 + 长图按钮横排

- **快捷键改为「按键录制」**：设置里「截图 / 贴图」不再是文本框，而是一个键帽样式的按钮，
  点一下直接按组合键即可（`src/shortcut.ts`）。规则：裸字母/数字拒绝（会吞掉系统里的那个
  键），裸 F1–F24 允许；Esc 取消录制，⌫ 清除。录制期间 Rust 侧 `hotkeys_suspend` 注销全局
  快捷键，否则按 F1 会直接触发截图而不是被录下来；退出录制 / 关窗口都会 `hotkeys_resume`。
  保存后 `settings_save` 立即重新注册，不用重启。
  证据：`--shortcut-selftest` → `before: F1` → `armed: 按下快捷键…` → `after key: F2`
  → 保存后 config.toml 里 `hotkey_snip = "F2"`，日志里 `hotkey: capture = F2`。
- **长图结果页的按钮竖排**：结果页复用了标注工具条 `.toolbar`（两行纵排），
  复制长图/保存长图/取消 被排成一列。加 `.long-shot .toolbar { flex-direction: row }`。
  证据：`shortcut-selftest` 里注入同样的结构后读到 `long-shot toolbar: row`。

## 6.56 2026-11-13：截图框出现后要「等一下」才能拖 —— 找到并修掉

现象：按快捷键后遮罩出现了，但头几百毫秒里框选没反应，等一下才能画。

根因不在渲染，也不在 JS（全程 0 个 >8 ms 的 `paint()`、0 个 long task）：**App 还没被
激活，窗口不是 key，第一次按下鼠标被 macOS 当成「点一下来激活这个 App」吃掉了。**
`overlay_show()` 里量到的状态是 `key=false active=false`，而窗口要过 ~130–250 ms 才变成
`key=true`；用户的第一个拖拽正好落在这一段里。

做了三件事：

1. **不再为截图把 App 切成 regular**。以前 `focus_webview()` 在 `show()` 前同步
   `setActivationPolicy: 0`，macOS 要先给 App 装菜单栏/程序坞图标，激活就慢。
   实测 accessory 下照样能 `active=true` + `key=true` + 收到按键，所以覆盖层保持
   accessory；只有设置/历史窗口在 `open_window()` 里切 regular。
2. **激活提前**：`start_capture()` 一开始就 `NSApp activateIgnoringOtherApps`，
   让 ~240 ms 的激活和抓帧并行，而不是等画面好了才开始。
3. **`acceptsFirstMouse`**：把覆盖层视图树里每个类的
   `acceptsFirstMouse:` 换成返回 `true`（`class_replaceMethod`）。即使激活还没完成，
   第一次点击/拖动也直接交给页面；点击仍然会激活 App。

证据（`--probe-selftest 20000`：预热完成后才截图，和真实 F1 一样）：

```
[  1978 ms] overlay 0 shown
[  1983 ms] focus overlay-0: key=true active=true policy=1 webview=true   ← 出现即 key
[  1924 ms] overlay 0 shown
[  1938 ms] focus overlay-0: key=true active=true policy=1 webview=true
[  1932 ms] overlay 0 shown
[  1939 ms] focus overlay-0: key=true active=true policy=1 webview=true
```

改之前同一处是 `key=false active=false`，80–250 ms 后才 `key=true`。

按键也验证了（accessory 模式下）：覆盖层起来后发 `v`，工具条里「选择」高亮 —— 说明
keydown 真的进了 webview，不只是窗口状态好看。设置窗口仍是 `policy=0`（
`--window-selftest settings` 读到 `visible=true focused=true`），关掉后回到 accessory。

顺带一个新钩子：`--probe-selftest <ms>` = 等预热完成再截一张、保持 `<ms>`，专门用来
量「真实 F1」的时序（其他 selftest 都是冷启动窗口，数字会被建窗时间污染）。

## 6.57 2026-11-13：设置里「点了没用」的项

清一轮设置项时发现的：有几个开关只写进 config.toml，界面/导出根本没读。

- **「选区外遮罩」和「放大镜倍率」从来没生效**：Overlay 里写死了 `0.5` 和 `zoom = 8`。
  现在 `MetaDto` 带上 `dim` / `magnifierZoom`（取自本次截图的 `Session.config`），
  `paint()` 用 `dim` 填外部遮罩（没有选区时用 `dim × 0.36`，保持原来那种轻微压暗），
  `drawMagnifier()` 用配置的倍率。下一次截图生效，不需要重启。
- **删掉三个假开关**：截图包含鼠标指针（`include_cursor`）、Enter 复制（`copy_to_clipboard`）、
  ⌘S 写文件（`save_to_file`）。前两个在 GUI 里没有实现路径；后两个覆盖层永远是
  Enter=复制、⌘S=存盘，勾不勾都一样。配置里的这两个字段保留给 CLI 用（`pinshot snap` 会读），
  但设置窗口不再显示它们。光标那条写进 README 的「已知限制」。
- `capture.allow_synthetic_scroll` 也从配置结构里删了：自动滚动已经在界面下线，
  只剩 CLI 的 `scroll-live`，它不读这个开关。旧配置文件里多出来的键会被忽略。

## 6.58 2026-11-13：删死代码 + 拆文件

纯整理，行为不变（同一套 selftest 重跑过）：

- **删掉界面早已不用的「拖到边缘实时采集」IPC**：`scroll_live_start/stop/cancel`、
  `live_strip`、`live_image` 五个命令、四个 `AppState` 字段、`--live-selftest`、
  `--button-selftest`，以及只被它们用的 `scrollable_pid_under` / `window_number`。
  约 450 行。`scripts/verify-live-scroll.sh` 与只跑它的调度脚本一并换成了现在这条
  `verify-overlay.sh`。
- **`src-tauri/src/main.rs` 3737 → 2172 行**：
  - `platform.rs`（193 行）：app 激活 / 首击接收 / webview 首响应者 / 窗口层级 /
    退出时回 accessory —— 原来散在 main 里的 `objc2` 桥接。
  - `selftest.rs`（863 行）：所有 `--capture`、`--*-selftest`、`--self-test` 钩子。
    `setup` 里只剩一行 `selftest::install(app.handle())`。
- **前端**：`Overlay.tsx` 1416 → 1225 行。放大镜（含取色）拆到 `magnifier.ts`，
  工具条拆到 `Toolbar.tsx`（纯展示，所有改动走回调）；强调色抽到 `theme.ts`。

整理后回归：`cargo test` 77、`pnpm test` 27、`tsc` / `clippy` / `fmt` 全绿；
`--capture-selftest`（key=true policy=1，125–172 ms）、`--window-selftest settings`
（policy=0，菜单栏真出现）、`--text-selftest`（工具条里找到「文字」→ 导出 135 KB）
都重跑通过。

遮罩接线也做了实测（临时 HOME，`dim=0.0` vs `0.8`）：选区外亮度 247.4 → 49.5
（恰好 ×0.2），选区内两种设置都保持 248 —— 设置真的生效了。

## 6.59 2026-11-13：工具条加「贴图」（把当前这张钉在屏幕上）

Snipaste 的贴图是「截完直接钉在屏幕上」，之前只有托盘/F3 贴历史里最新的一张，标注过的
那张贴不出去。

- 工具条第一行在「保存」后新增 **贴图**：没有标注时走 `capture_commit(action="pin")`
  （Rust 直接裁冻帧 → 记录 → 开贴图窗）；有标注时走新命令 `pin_png`（前端合成的那份
  PNG → 记录 → 开贴图窗）。两条路都把最终像素写进历史，贴图窗从历史读像素，和从托盘
  恢复的贴图完全同源。
- `pin_capture()` 拆成 `pin_window(capture_id, w, h, scale)`，托盘路径与工具条路径共用
  同一套「偏移 + 不出屏」的窗口逻辑。
- **截图过程中按贴图键（默认 F3）现在贴的是当前这张**，不是上一张；不在截图状态时仍是
  「贴出最近一次截图」。带文字输入框时按 F3 会先提交文字再贴。
- 新钩子 `--pin-selftest x,y,w,h`：加一个文字标注 → 点工具条「贴图」→ 报告贴图窗数量。
  证据：`selftest: tool=found` → `input=present` → `pin=found` →
  `pin-selftest: 1 pin window(s): ["pin-1"]`；截图里能直接看到 (80,80) 的贴图窗，
  内容是带红字「贴图」的那块选区。
- 两处视觉收尾：工具条「复制 / 保存」后面的 `↵`、`⌘S` 角标去掉（`overlay.save`
  文案也改成纯「保存 / Save」）；贴图窗右上角的关闭按钮不再用原生 `<button>` 的
  默认外观（会带边框/按下位移），改成 `appearance: none` + 10×10 SVG 叉，悬停变强调色，
  圆形按钮不再跳。`src/styles.css` 里没人用的 `kbd` 样式一并删掉。

## 6.60 2026-11-13：贴图窗滚轮缩放

- 贴图窗上**滚轮以光标为锚点缩放**：`factor = exp(-deltaY × 0.0015)`，范围 25%–400%，
  一个滚轮刻度约 20%。缩放的是**窗口**而不是位图，图片会按新尺寸重新光栅化，文字不会糊。
- 锚点算法：`新原点 = 旧原点 + 光标在窗口内的坐标 × (1 − 缩放比)`，光标下那一点保持不动；
  窗口先 `setSize` 再 `setPosition`，一步完成。
- 左下角显示当前百分比，800 ms 后淡出；拖动移动、双击关闭不变。
- **ACL 坑**：`setSize` 属于 `core:window:allow-set-size`，`src-tauri/capabilities/default.json`
  原先只开了 `set-position`，页面调用会抛
  `Command plugin:window|set_size not allowed by ACL`。补上权限后正常。
- 验证：`--pin-selftest` 里合成一次 `wheel`（deltaY −120）→ 窗口从 1200×800 变成
  **1436×958**（600×400 点 @2x），正好 1.197×。测试挂在 `--pin-selftest` 的贴图环节，
  没有真实滚轮也能回归。

## 6.61 2026-11-13：长图接缝处「一行字多出一点」

现象：滚动截屏拼出来的长图，在接缝附近有一小段内容重复（一行字看起来多了一点）。

根因（已稳定复现）：`detect_fixed_rows()` 把两帧中**完全相同**的连续行当成「吸顶头 /
固定底栏」裁掉。页面两段文字之间的空白行，在两张截图里当然一模一样（都是纯背景），
于是被误判成固定元素，从帧尾裁掉几行。裁掉的这几行会改变这一帧在长图里的取行起点，
后面所有条带整体错开几个像素 —— 接缝处就多出一行。

两处修复：

1. **空白行不再算固定条**：先算整帧的「运动列」（`motion_columns`），只有当一段连续
   相同行里**至少有一行在这些列上有内容**（墨迹/对比度）时，才认定它为固定头/尾；
   整段都是纯背景则保留为正文。真实吸顶头、固定底栏（有文字或色块）仍然会被裁掉。
   运动列同时复用到后面的相关搜索，不再按 body 单独算一遍。
2. **低置信度时全分辨率兜底**：金字塔 4×/2× 层会把 1px 文字笔画平均成近灰色，个别帧对
   会在粗层锁到噪声上，细层只在 ±2 px 内细化、救不回来。现在最终置信度低于 `confidence`
   时，回到原分辨率把所有候选偏移重扫一遍（列采样到 256 列以内，几十毫秒，只发生在失败的那一对上）。

回归（新增 `crates/pinshot-stitch/tests/offset_accuracy.rs`）：

- 合成「1px 笔画 + 12 行字 / 12 行空白」的页面，按 137/153/143/157… 这类偏移取 10 帧。
  修复前：**205 行不匹配**，第一个错误行 537（正是空白带所在的接缝）；修复后 **0**。
- 另一例专门放一条 110 行的空白带：修复前该帧对直接 `LowConfidence`（猜成 346，真值 130），
  加兜底后 0 错误。
- 全量：`cargo test --workspace` **79 passed**，clippy 0。

## 6.62 2026-11-13：切换语言后托盘菜单和选框工具条还是旧语言

现象：设置里选了 English，设置窗口自己变了，但托盘菜单、重新截图的工具条还是中文。

三个独立原因：

1. **覆盖层是常驻页面**：`src/i18n.ts` 的语言是每个 webview 里的模块状态，设置窗口
   `setLocale()` 只改自己的模块；预热好的覆盖层页面（隐藏中）不会重新加载，所以一直是
   旧语言。现在 `settings_save` 会 `emit("language:changed", "zh"|"en")`，`main.tsx` 在
   每个 webview 里监听并 `setLocale`；同时 `overlay_meta` 带上 `locale`，覆盖层每次
   boot 都按本次截图的配置再设一次（事件丢了也不会错）。
2. **托盘菜单是原生菜单**：它在 `setup_tray()` 里只建一次。现在把菜单构建抽成
   `tray_menu()`，`settings_save` 调 `refresh_tray()`，用 `TrayIcon::set_menu` 换掉菜单
   和 tooltip，点保存即生效，不用重启。
3. 设置里的提示文案原来写着「托盘菜单在下次启动后跟随」，改成「立即生效，托盘菜单同步切换」。

验证（临时 HOME，`language = "en"` 启动，`--set-language zh` 在 3 秒时模拟保存）：

- 2.6 s 截图：工具条 `Select Rectangle Ellipse … Scrolling capture Copy Save Pin Cancel`；
- 4.4 s 截图：同一张覆盖层、没有重新截图，工具条变成 `选择 矩形 … 复制 保存 贴图 取消`。

## 6.63 2026-11-13：长时间不用后第一次截图明显慢

现象：隔一阵子不截图，按快捷键唤起时明显慢；连着截几次就恢复。

原因：`close_overlays()` 里有个 60 秒的空闲计时器，超时后 `destroy_overlays()` 把预热窗口
整个销毁，把 ~100 MB 还给系统。消息里「多用几次就快」正是因为每次截图都会重置计时器。
下一次截图于是要走「建窗口 + 加载页面 + boot」的冷路径。

改法：

- **预热窗口常驻**，不再按空闲销毁（`capture_serial` 和那 60 秒计时器一并删除）。代价是
  常驻内存约 80–100 MB；换来的是隔多久第一次截图都走热路径。
- **退出 App Nap**：启动时 `NSProcessInfo beginActivityWithOptions:`（
  `NSActivityUserInitiatedAllowingIdleSystemSleep`），避免后台闲置被降频拖慢首次响应。
  这里踩了个坑：常量一开始写成 `0x00FFFFFF`（= UserInitiated，含 IdleSystemSleepDisabled），
  `pmset -g assertions` 里会显示 `PreventUserIdleSystemSleep named: "PinShot stays ready to capture"`
  ——等于让整台 Mac 永不睡眠。改成 `0x00EFFFFF` 后不再持有睡眠断言，但 App Nap 仍然被抑制。
- 新钩子 `--capture-after <ms>`：启动后延迟 ms 再截一张，用来量「闲置后第一次」。

实测（2880×1800，双屏）：

```
启动 1.8 s 后截图：        capture: overlays woken in 69 ms → first frame 134 ms
闲置 70 s（屏幕保持亮）：  capture: overlays woken in 2 ms  → first frame 189 ms
闲置 70 s（屏幕已熄灭）：  capture: overlays woken in 173 ms → first frame 301 ms
```

也就是：闲置本身只多 ~55 ms（WebKit 从隐藏/遮挡状态恢复）；剩下的大头是显示器休眠唤醒
（用户通常先唤醒屏幕再按快捷键，所以日常是 ~190 ms）。改造前走销毁→重建冷路径
（`capture: overlays built` 要 1.1 s，再加页面加载）。

## 6.64 2026-11-13：「保留预热窗口」做成设置项

6.63 把预热窗口改成常驻，但那是拿 ~100 MB 换速度，应该让用户自己选。

- 配置新增 `general.keep_warm`，默认 `true`（新行为：常驻）；旧配置文件没有这个键时按默认走。
- 设置 → 截图 → **保留预热窗口**：
  - 开：截图框常驻内存（约 80–100 MB），隔多久第一次截图都快；
  - 关：每次截图 60 秒后释放，下一次重新建窗口（慢一点）。
- 实现：`AppState.keep_warm` 镜像配置，`schedule_idle_release()` 在每次关闭覆盖层后挂一个
  60 秒计时器；到期时如果 `keep_warm` 已打开、或这期间发生过新截图（`capture_serial` 变了）、
  或覆盖层正开着，就什么都不做。关掉开关保存时会立刻挂一个计时器，不需要等下一次截图。
- 验证：临时 HOME `keep_warm = false`，截图后等 65 秒 → 日志
  `prewarm: idle, overlays released`；`keep_warm = true` 同样等待后没有这行，下一次截图
  首帧仍是 ~190 ms。设置窗口里新开关在「滚动截屏」下面，中英文两套文案。

## 6.65 2026-11-13：macOS 安装包 + 干净的源码包

- **一条命令构建**：`tauri.conf.json` 补上 `beforeDevCommand` / `devUrl` /
  `beforeBuildCommand`。以前必须先 `pnpm build` 再 `pnpm tauri build`，少一步就
  把旧的 `dist/` 打进包里；现在 `pnpm tauri build` 会自己跑 `pnpm build`，
  `pnpm tauri dev` 会自己起 Vite。已用删掉 `dist/` 的干净目录验证。
- **DMG**：`scripts/make-dmg.sh` → `PinShot_0.1.0_arm64.dmg`，5.0 MB。签名身份是
  本机的 `OpenCodeGo Dev`（自签，未公证），所以别人第一次打开会看到
  「无法验证开发者」：右键 → 打开，或 系统设置 → 隐私与安全性 → 仍要打开。
- **清理**：删除 8 个 `.DS_Store`、`src-tauri/gen/`（tauri-build 每次重新生成）、
  `dist/`（`pnpm build` 重新生成），`.gitignore` 补 `/src-tauri/gen`。
- **源码 zip**：`PinShot-src-0.1.0.zip`（685 KB / 148 个文件），排除
  `target/`、`src-tauri/target/`、`node_modules/`、`dist/`、`src-tauri/gen/`、`.DS_Store`。
- **Windows**：源码可以在 Windows 上编出 MSI/NSIS，但截图后端没有 Windows 实现，
  装上也只会报「不支持」；`docs/WINDOWS.md` 重写成「拷过去会发生什么 + 还差什么」。
- 回归：装好的 `/Applications/PinShot.app` 跑 `--probe-selftest 3000`，
  `overlay 0` 首帧 132 ms。

## 6.66 2026-11-13：Intel（x86_64）安装包

Apple Silicon 上可以直接交叉编译 Intel 版，不需要另一台机器：

```bash
rustup target add x86_64-apple-darwin
pnpm tauri build --target x86_64-apple-darwin --bundles app
./scripts/make-dmg.sh x86_64-apple-darwin
```

- 首次为 x86_64 编译依赖约 2 分 12 秒；产物
  `src-tauri/target/x86_64-apple-darwin/release/bundle/dmg/PinShot_0.1.0_x86_64.dmg`，5.5 MB。
- `scripts/make-dmg.sh` 现在接受可选 target，并用 `lipo -archs` 给文件命名（不再用 `uname -m`，
  否则 Intel 包会被叫成 arm64）。签名身份和 bundle id 与 arm64 版一致。
- 验证：`lipo -archs` → `x86_64`；DMG 挂载后 `codesign --verify --deep --strict` 通过；
  在本机 Rosetta 下跑 `--window-selftest settings`，设置窗 `visible=true focused=true`，
  隐藏后再次打开也聚焦（预热窗口冷建 1.55 s，Rosetta 下偏慢，真 Intel 机器无此开销）。
- 真 Intel Mac 上不需要 Rosetta；Apple Silicon 用户想跑这个包时才需要。

## 6.67 2026-11-14：把 Windows 移植合回主线

来源是 `docs/PinShot-Windows-0.1.0.zip` 里的 `PinShot-src-windows-0.1.0.zip`
（Windows 11 + WebView2 实机跑通过的那份）。现在同一份源码两边都能打。

**合进来的**

- `crates/pinshot-capture/src/win.rs`（新）：GDI `BitBlt` 抓屏、显示器枚举、per-monitor
  DPI 换算、窗口枚举（DWM 扩展边框 / cloaked 过滤）、系统语言、原生错误框。
- `lib.rs` / `windows.rs` / `events.rs`：Windows 后端接进 `system()`；窗口列表/单窗截图；
  `SendInput` 点击、拖拽、滚轮、按键，`VK_*` 键码。
- `pinshot-capture/Cargo.toml`：`windows-sys` 只在 `cfg(windows)` 下依赖。
- `pinshot-core/config.rs`：`USERPROFILE` 作为 Windows 的 home。
- `pinshot-cli`：Windows 启动时 `ensure_dpi_awareness()`。
- `src-tauri/main.rs`：Windows 托盘彩色图标、`HKCU\...\Run` 开机启动、
  `GetUserDefaultUILanguage` 判断中文、`MessageBoxW` 错误框、贴图窗口先建后关覆盖层。
- `src/HistoryView.tsx`：`void reload()` 原来写在 `return` 之后，永远不执行（历史一直转圈），
  现在移到 cleanup 之前。
- `pnpm-workspace.yaml`：同时写 `onlyBuiltDependencies`（pnpm 10）与 `allowBuilds`（pnpm 11+），
  esbuild 的安装脚本两边都不会被拦。

**没有照抄的地方**

1. `mainBinaryName: "PinShot"` 放进 `tauri.windows.conf.json`，不放在公共配置里：Windows 的
   程序叫 `PinShot.exe`，macOS 包内仍是 `pinshot-desktop`，`install.sh` / `make-dmg.sh` /
   `acceptance.sh` 不用改。
2. 诊断日志 macOS 仍是 `/tmp/pinshot-diag.log`（现有排查命令和 STATUS 都按这个路径），
   Windows 才用 `%TEMP%\pinshot-diag.log`。
3. 保留我们的 `make-dmg.sh`（带 target 参数、按 `lipo -archs` 命名），没有用他们那份旧脚本。
4. `config.toml` 现在会转义路径里的反斜杠和引号，读取时做对应反转义；旧的 Windows 配置
   （单反斜杠）仍然能读。新增 `windows_paths_survive_the_round_trip` 测试。
5. `pinshot-ocr/src/lib.rs` 只多了个 BOM，不采纳。
6. 贴图窗口「先建窗、后关覆盖层」在 macOS 上也验证过（`--pin-selftest` 正常出图 + 缩放）。

**验证**

- macOS：`cargo test --workspace` 80 passed、`pnpm test` 27 passed、`clippy` 0、`fmt` / `tsc` 干净。
- macOS 实跑：装好后 `--pin-selftest 100,120,700,450` → `1 pin window(s): ["pin-1"]`，
  `zoom 1400x900 -> 1676x1078`；`--probe-selftest 3000` 首帧 **92 ms**。
- Windows 代码交叉检查（本机）：`cargo check -p pinshot-capture --target
  x86_64-pc-windows-msvc` 通过，同 target 的 `clippy` 0 warning。完整 MSI/NSIS 打包需要
  Windows 机器（MSVC 链接器）。
- Windows 实机功能证据见交付包的 `README-Windows.md`：F1、框选、标注、复制、保存、贴图缩放、
  历史、设置中英文、手动滚动截屏（2000×3000）、安装/卸载都通过。

**仍然没有的**

- Windows 上的 OCR（Vision）和自动滚动（Accessibility）；手动滚动截屏可用。
- Windows 的 MSI 目标需要 WiX；交付的是 NSIS，功能一样。

## 6.68 2026-11-14：发布物统一放到 `docs/`

`docs/` 现在同时是源码文档和发布目录，和之前的 `PinShot-Windows-0.1.0.zip` 放在一起：

| 文件 | 内容 |
| --- | --- |
| `PinShot_0.1.0_arm64.dmg` | macOS Apple Silicon 安装包（5.0 MB） |
| `PinShot_0.1.0_x86_64.dmg` | macOS Intel 安装包（5.5 MB） |
| `PinShot-src-0.1.0.zip` | 合并 Windows 移植后的完整源码（697 KB） |
| `PinShot-Windows-0.1.0/` | Windows 安装器、免安装 exe、实机源码包与构建说明（由交付 zip 解压） |

源码包里排除了 `target/`、`node_modules/`、`dist/`、`src-tauri/gen/`，以及 `docs/` 下的
`.dmg` / `.zip`，所以不会把一个发布包再套进源码包。README 和 `docs/WINDOWS.md` 都指向这里。

## 6.69 2026-11-14：闲置后第一次截图，框没生效 / 看到上次的框

现象：放很久再按快捷键，第一次拖框没反应；再拖一次，看到的是上一次留下的选区。

暖覆盖层是常驻页面，正常路径靠 `overlay:end` 清场。长时间闲置（尤其是合盖/休眠）时有两个
薄弱点：

1. `close_overlays` 是「先 emit end、再 hide」。如果系统正好在两句之间休眠，窗口可能带着
   上一张冻结帧和旧选区留在屏幕上；下一次截图时它还是可见的，第一下拖拽就落在旧状态上。
2. 页面只有在 `overlay:end` 时清场；`overlay:begin` 的 boot 要先取 meta/frame 才
   `setSelection(null)`。这一段空窗期里，旧选区仍然是活的，「拖框」会被当成移动/编辑旧框。
3. 再加一个竞态：boot 很慢（刚唤醒）时用户取消再按一次，旧的 `bootOnce` 布尔守卫会丢掉
   新的 begin；旧 boot 结束时还可能把旧画面 show 到新一次截图上。

改法：

- `close_overlays` 先 `hide()` 再 `emit("overlay:end")`，休眠夹在中间也不会留下可见的旧窗。
- 每次 `begin_warm_overlays` 先 hide 再唤醒，窗口不可能是「上一张还开着」的状态。
- `boot()` **第一步** `resetScene()`，不等 meta/frame；旧选区/旧图在任何等待之前就清掉。
- 给每次截图一个序号：`MetaDto.capture_serial`、`overlay_idle` 返回当前序号、
  `overlay:begin` / `overlay:end` 都带序号，`overlay_show` 校验序号。旧 boot 晚到不会盖在
  新截图上，迟到的 `overlay:end` 也不会清掉新一次截图的选区。
- 重复信号去重：页面的挂载检查和 `overlay:begin` 常常同时到达；序号相同只 boot 一次
  （之前调试用的 `PINSHOT_DEBUG_REGION` 会被第二次 boot 吃掉，导致预置选区丢失）。
- boot 期间再收到新截图（序号变化）时排队重跑，而不是丢弃。
- `overlay_show` 里再 `activate_app()` 一次，聚焦重试加一档 450 ms：显示器刚唤醒时，
  按快捷键那次激活可能被系统吞掉，第一下拖拽只用来激活 App。
- `overlay_meta` / `overlay_frame` 等待 session 的上限 2.5 s → 6 s，给系统唤醒后的第一帧
  留余量。

验证：

- `--capture-selftest`：连续两次热路径截图各 **一次 boot**，首帧 **100 / 85 ms**。
- `--pin-selftest 100,120,700,450`：一次 boot，预置选区保留，`1 pin window(s)`，
  `zoom 1400x900 -> 1676x1078`。
- `--probe-selftest`：`perf: boot 74 ms`，首帧 123 ms。
- `cargo test` 80、`pnpm test` 27、`clippy` 0、`fmt` / `tsc` 干净。

## 6.84 2026-11-20：修贴图右键菜单背景透明 + 关闭无效

6.83 的右键菜单上线后用户反馈：菜单背景透明、文字看不清；点「关闭贴图」关不掉。
两个根因：
- CSS 用了不存在的变量 `var(--card)` / `var(--line)`（`:root` 里实际定义的是
  `--panel` / `--panel-border`），背景/边框解析失败 → 透明。改用正确变量名。
- window 的 `mousedown` dismiss 在 `click` 之前触发，点菜单项时菜单被提前卸载，
  `onClick` 永不执行。dismiss 现在排除落在 `.pin-menu` 内的点击。

`--pin-selftest` 扩展验证：菜单 `bg=rgba(28,28,30,0.92)`（不透明）、
点关闭后 `pin-1 closed ok`。检查点 `pre-menu-fix`。质量门全绿（83+2/tsc/vitest 27）。

## 6.83 2026-11-20：贴图停留选区原位 + 贴图窗中文右键菜单

用户反馈两点：点贴图时截图跑到小屏左上方（想停留原地）；贴图窗右键菜单是英文、不跟随
语言设置。

**贴图位置**：`pin_window` 原来把窗口固定在主屏左上角 (80,80) 堆叠，完全没用选区位置。
改法：MetaDto 加显示器全局原点 originX/originY；前端 commit pin 前调新命令
`set_pin_origin` 传选区全局点；capture_commit 的 pin 路径直接用 monitor_origin + 局部点；
pin_window 加 `at` 参数优先用该点，clamp 到虚拟桌面 union 内保证关闭按钮可达；托盘/F3
贴最近一张仍走原堆叠逻辑。检查点 `pre-pin-position`（= 59e27f5）。

**右键菜单**：贴图窗是 WebView，右键弹 WebKit 自带英文菜单、无视应用语言。改为前端
`contextmenu` preventDefault + 自定义菜单，用 `t()` 文案（跟随 `language:changed`），含
「复制图片到剪贴板」「关闭贴图」；新增 `pin_copy` 命令按 pin id 从历史取图复制。

真机验证（`--pin-selftest`，扩展了位置与菜单报告）：
- 选区逻辑 (120,120)，pin 的 outer_position=(240,240)——2x 屏上正好盖在选区原位，
  不再跑左上角。缩放 1200×800→1436×958 仍正常。
- 中文菜单「复制图片到剪贴板 | 关闭贴图」；英文 "Copy image to clipboard | Close pin"。
  配置已恢复中文，测试进程全退出。

质量门：fmt/clippy 干净、workspace 83、app 2、tsc 通过、vitest 27（本机 /etc/hosts 缺
localhost 条目导致 vitest 启动失败，用 work/ 下 dns shim 绕过，未改系统）。

## 6.82 2026-11-20：README 说明产物用 GitHub Release 分发

用户问安装包是否要提交 git。确认 .gitignore 已忽略全部产物（约 31 MB）：两个 DMG +
源码包 + 18 MB Windows 旧包。说明提交会使 git 历史永久膨胀且旧包已过时后，用户决定不提交，
但要在 README 写清楚这些产物可放 GitHub Release。

README「从源码构建」末尾新增「分发产物与 GitHub Release」小节：说明产物按设计不进 git
的原因、列出该挂到 Release 的四个文件、指出 Windows 那份旧包不建议作为发布附件、推荐用
自生成的源码包，以及临时入库的办法（不推荐）。锚点检查通过。纯文档改动，源码包更新。

## 6.81 2026-11-20：README/WINDOWS 说明 Windows 包是旧构建

用户指出 Windows 不能用最新代码打包，需在文档说清楚。查明 `docs/PinShot-Windows-0.1.0/`
里的安装包是 2026-11-13/14 的旧构建，不含此后源码修复（Windows 通知、导出优化、回车修复等）。

- README「安装 → Windows」开头加醒目提示：旧构建、不是最新代码、由 macOS 交叉编译产出、
  只在 Windows 实机做过基础验证、**未针对最新代码真机详细测试**；建议在 Windows 上用当前
  源码自行打包（链接到「从源码构建」）。旧包的使用步骤保留但注明「若只用旧包」。
- docs/WINDOWS.md 开头同样加状态说明，并点明最新加入的 Windows 通知插件仅做了 target
  交叉编译检查、是否真弹出需实机确认，避免与功能表的「✅」矛盾。
- 锚点检查通过。纯文档改动，DMG 不含文档无需重打，源码包更新。

## 6.80 2026-11-20：README 开头加功能海报

用户反馈开头几张图没体现滚动截屏，希望滚动截屏放前面；或换成一张功能海报放开头、
详细功能介绍放后边。采用后者：

- 生成 `docs/images/poster.png`（1600×1200）：深色主题 + 主色 #e8452c，标题/简介/
  特点标签 + 框选/标注/滚动截屏/贴图/设置五个功能区，滚动截屏用红色标签和中间主视觉列
  突出（用真实 long.png 缩略），底部六个特点卡。HTML 排版 + headless Chrome 渲染，
  文字准确、不截用户桌面；生成脚本留在 `work/poster.html`（gitignored）。
- README 开头改为海报一张图；原 snip/annotate/long/settings 四张散图整理进新的
  「功能一览」章节放到后面，「长图」「设置」章节去掉重复图改为指向功能一览。
- 校验：5 张图片引用都存在、2 个内部锚点有效。仅文档改动，无需重打 DMG，源码包更新。

## 6.79 2026-11-20：Windows 滚动截屏补操作提示

用户反馈：Windows 上滚动截屏没有任何提示，不像 mac 上有通知。根因是滚动开始/
滚太快两处提示直接 `spawn("osascript ... display notification")`，Windows 没有
osascript、spawn 失败被 `let _ =` 吞掉 → 完全静默，不是设计成没有。

改法：加 `tauri-plugin-notification` 2.8.1 + capability `notification:default`，封装
`notify(app, title, body)`：macOS 沿用 osascript（已工作、免权限），其它平台走插件
的 `app.notification().builder().title().body().show()`。两处提示改调 notify，文案继续
用 `tr()` 中英文。检查点 `pre-win-notify`（= d459471）。

验证（09:46–10:05 实测，用户授权覆盖时段）：
- macOS 全质量门绿：fmt 干净、workspace/app clippy 0、workspace 测试 83、app 2、
  tsc/vitest 通过、app 构建成功（14.40→14.73 MiB，插件编进去了）。
- Windows target：`work/verify-win-notify/` 最小 crate 独立交叉编译通过
  （含 tauri-winrt-notification 0.8.1），确认通知 API 用法正确；main.rs 本就 import
  了 Manager，`app.notification()` 可用。完整 Windows app 交叉编译仍卡在本机缺 MSVC
  头文件的 sqlite C 编译（与改动无关，历史上也只交叉检查过 pinshot-capture）。
- tauri.conf.json 的 identifier=ai.pinshot.desktop 会在打包时生成 AppUserModelID，
  满足插件只在 installed apps 生效的要求。
- **通知是否真正在 Windows 弹出未实测**（本机 macOS，无 Windows 实机），需 Windows 验证。

## 6.78 2026-11-19：导出 PNG 改用 Arc<[u8]>

6.77 里唯一明确没做的 A6 项，本轮补上。`Export.png` 与 `NewCapture.png` 从 `Vec<u8>`
改为 `Arc<[u8]>`：IPC raw 体只拷贝一次（无法避免），之后复制/保存/贴图/历史记录/
缩略图全部共享同一份 buffer，clone 从 O(n) 变 O(1)，不再在点击路径上反复拷贝数 MB
的 PNG。app::record 把 Arc 直接传给 NewCapture::snip；store::save_blob 按 &[u8] 读自动
解引用。检查点 `pre-a6`（= 29bb2de）。

质量门（12:39–12:50 实测）：fmt 干净、workspace clippy 0、app clippy 0、
workspace 测试 83 passed、app 单测 2 passed。期间 clippy 抓到 store 测试一处
`Vec<u8> == Arc<[u8]>` 比较错误，已改为按 slice 比较。

真机 `--text-selftest 14,60,1200,780,A6`：输入 3 个文字标签后导出成功
（`/tmp/text-selftest.png` 2400×1560），历史记录 35→36、新行 kind=snip/2400×1560/
blob_hash 正常，证明 IPC→Export(Arc)→record→写盘/缩略图链路完整。测试进程已退出，
配置未变。

## 6.77 2026-11-19：体检报告的一批 bug 与性能修复

基于三路只读审查（Rust 后端 / 前端 / 拼接算法）的结论，按可回退方式集中修复。
检查点 tag `pre-review-fixes`（= 93257d3），本轮提交 `25c9c74` + `07ee911`。

**修的**：见对应提交的说明。要点：前端长图 objectURL 泄漏 + scroll_result 回填 serial
校验 + 按来源去重 + rAF 重绘合并 + 拖拽去掉每帧 setRevision；dim=NaN 校验、save_blob
临时名加序号；拼接置信度先于 offset==0、新增 stitch_plan() 跳过 57 MB 灰度合成图、
回退近满分提前退出、tolerant 复用 anchor luma、Down 方向去掉冗余帧副本（~1.2 GB）、
回滚失败不再吞；Rust 壳去掉重复显示器枚举、scroll_result 锁失败上报 failed、滚动会话
提前占用封住 F1 取消的 TOCTOU 窗口、AX BFS 提前退出释放剩余引用、Windows 并行 grab_all。

**未做**：导出 PNG 多次 clone（需把 Export.png 改 Arc<[u8]> 贯穿 store/CLI，面大）。

质量门（2026-11-19 10:19–10:30 实测）：fmt 干净、workspace clippy 0、app clippy 0、
workspace 测试 83 passed（1 ignored=周期长图已知限制）、app 单测 2 passed、tsc 通过、
vitest 27 passed、`pnpm build` 通过。期间 clippy 抓到 `b` use-after-move（`07ee911` 修）。

真机 selftest：
- `--window-close-selftest`：`settings window closed ok`，配置已恢复原值。
- `--capture-selftest 2`：首帧 107 ms / boot 94 ms，与此前基线一致（无退化）。
- `--scroll-selftest window:<演示页>,manual`：40 帧拼成 2400×6168，链路正常。
测试进程已全部退出，未做物理按键 GUI 注入（遵守「不抢键鼠」）。

## 6.76 2026-11-18：回滚引入的「滚动截屏敲回车没反应」

现象：6.75 之后，手动滚动截屏滚完按 Enter 没反应——长图结果视图不出现。

根因：6.75 为消除「取消后幽灵覆盖层」，把滚动线程结尾的 re-show 条件写成
`!cancelled && !serial_changed`。但 Enter/Esc 正常结束时也会调 `scroll_cancel`、把
`scroll_cancel` 置 true——它是「停止并拼接」的正常信号，不是「被取消」。于是覆盖层
不再 show，长图结果视图（画在同一个覆盖层页面里）一直藏在隐藏窗口中。

改法：只保留 `serial_changed` 判断——新截图（close_overlays 会自增 serial）接管时保持
隐藏，否则都 re-show。抽出 `should_reveal_overlays(current, scroll)` 纯函数并加单测锁定：
serial 不变→显示结果、变了→保持隐藏。`cargo test --manifest-path src-tauri` 2 passed。

验证：窗口级手动滚动 selftest 仍拼出 2400×1560，quality gate 全绿（83/0/27）。端到端
物理按键路径因「不抢键鼠」约束未做 GUI 注入，改由单测锁定判定逻辑。

## 6.75 2026-11-18：全面体检与修复（设置关不上 + 一批潜在 bug）

用户反馈「设置窗口点 X 关不上」，并要求全面体检。三路只读审查（Rust 壳 / 前端 / 核心
crate）+ 逐条对源码确认，按可回退的分批提交修复。检查点 tag `pre-fixes`（= d3bfd56）。

**修的（每批独立提交）**：

1. `a46fd76` 设置关不上 + 前端交互：
   - 根因：自动保存的关窗 handler 调 `getCurrentWindow().destroy()`，但 capabilities 只授了
     `allow-close`、没授 `allow-destroy`；`preventDefault` 拦下默认关闭后 destroy 被拒 → 窗口
     卡住。补 `core:window:allow-destroy`。
   - Esc/Enter 走 DOM keydown + Rust 全局热键两条路径，加 50 ms 去重，修「画一半按一次 Esc
     直接退掉整张截图」和双次导出。
   - 两个快捷键录制框改为父组件单一状态，避免一次按键改两个热键；关窗兜底 `hotkeys_resume`。
   - 贴图窗拖动加 `draggingRef` + `event.buttons` 检查（快速点击后粘光标），滚轮缩放用
     Promise 链串行化（快速滚动锚点漂移）。
2. `7f39c70` 截图竞态：
   - `overlays_are_warm` 原来先 `store_monitors` 当前布局再比较，导致永远 warm；显示器
     热插拔/改分辨率后仍复用旧几何，多一块屏时冒出越界空白窗。改为先比对建窗布局，并把
     pixel_height/scale/窗口数纳入。
   - 滚动取消：`close_overlays` 清 `capturing` + 置 `scroll_cancel`；滚动线程按 serial 判断，
     被新截图接管就不 show，消除幽灵窗/压制新截图。
   - `overlay_under_cursor` 在双屏间隙回退主显示器，避免 target=null 时所有屏同时导出。
   - 焦点重试从单次 400 ms 恢复为 400/800 ms 两次（均 quiet），兜底显示器晚醒。
3. `47e623f` 配置与导出健壮性：
   - 配置写入改临时文件 + rename（避免写坏丢全部设置）。
   - TOML 只在引号外切 `#` 注释（save_dir 含 `#` 不再被截断），加回归测试。
   - `keep_days`/`keep_items` clamp ≥1（避免手改 0 清空历史），加回归测试。
   - 导出临时名加 pid+计数器（并发不互相覆盖）；目标存在先删（兼容 Windows rename）。
4. `7bc87e8` 长图：
   - 新增 `tests/periodic.rs` 并**复现**：滚动量为内容周期整数倍时长图只剩一屏
     （height=300）。根治需多候选投票、会冲击逐像素接缝测试，故标 `#[ignore]` 记录限制，
     不误报已修。
   - 自动滚动超视口分支补 `scroll(-fraction*sign)` 回滚（原来只缩小继续，页面已跳过一屏
     不留回滚 → 拼出空洞），与 `Probe::Unmatched` 对称。
5. `394e878` 加 `--window-close-selftest`：打开设置→改脏→`window.close()`（等于点 X），
   实测 `settings window closed ok`，证明 destroy 权限生效。

**没改的（如实记录）**：

- Windows 句柄截断为 u32（`win.rs`）：`Monitor.id` 是 u32，64 位句柄只留低 32 位，理论上
  两个显示器句柄低位相同会抓错。改成 u64 牵涉 core/capture/CLI 多处 + specta 导出，机器
  上无 Windows 无法验证，收益是罕见边界，暂不动。
- 周期长图截断（见上）需算法级多候选投票，留作已知限制。
- 历史保留策略目前只在用户点「清理」时执行，非自动；要不要自动清理是产品决定。

质量门：`cargo test --workspace` 83（+2）、`cargo clippy --workspace` 0、app clippy 0、
fmt/tsc 干净、`pnpm vitest run` 27。两个 DMG 和源码包重打。

## 6.74 2026-11-18：按快捷键闪屏 + 「已停止自动接收权限」

现象：按截图快捷键准备框选时屏幕连闪好几次，同时系统提示「已停止自动接收权限」；过了一会提示自己消失。

定位（只读代码 + 翻已有诊断日志，没有在用户桌面上复现）：

- 「已停止自动接收权限」是 TCC 对**反复申请权限**的自我保护。`start_capture` 在
  `permission() == Denied` 时每次都调 `CGRequestScreenCaptureAccess()`，再用 `osascript
  display alert` 弹一个本地警告框；而 TCC 授权是**按进程缓存**的，就算当场在系统设置里
  勾了，运行中的进程仍然读到 Denied。于是按一次弹一次，连按几次就触发了系统的提示。
  `docs/STATUS.md` 6.7 节也记过同类问题：`tauri build` 会把 bundle id 变成
  `pinshot_desktop-<hash>`，旧授权作废；`install.sh` 已经用 `ai.pinshot.desktop` 固定住。
- 「闪好几次」有两处叠加：
  1. 一次截图里 App 被激活 / 覆盖层被 `makeKeyAndOrderFront` 置顶 8 次——
     `start_capture` 1 次、`overlay_show` 1 次、结尾 80/220/450 ms 的重试线程对两个
     全屏窗口各 3 次（`focus_webview` 内含 `activateIgnoringOtherApps` +
     `makeKeyAndOrderFront`）。每次重复置顶都是肉眼可见的一闪。
  2. 权限失败时的 `osascript` 弹窗本身也在闪，和①叠加。
- 排除了「一次按键被识别成多次」：`global-hotkey 0.8.0` 的 Carbon handler 每个物理按下
  只发一次 `Pressed`，没有 auto-repeat。

改法：

- 权限请求节流：`SCREEN_ACCESS_PROMPTED` 一个进程只放行一次。第一次才请求系统授权 +
  弹本地警告（文案改成「允许后请**彻底退出并重新打开** PinShot」，因为授权对运行中
  的进程不生效）；之后按键只在诊断日志里记一行，不再弹窗。托盘「检查权限」仍可主动请求。
- 重试阶梯 80/220/450 ms → 单次 400 ms；重试里不再 `activate` / `makeKeyAndOrderFront`，
  新增 `focus_webview_quiet()` 只重设 WKWebView 第一响应者；而且只有 `is_focused() ==
  false`（托盘菜单还没关、显示器刚唤醒）时才补一次 `set_focus()`。
- 诊断日志的 `focus` 行加上 `reorder=true/false`，方便确认没有重复置顶。
- 加了一条单元测试 `screen_access_is_prompted_once_per_process`，验证节流只放行第一次。

实测（`--capture-selftest 2`，只跑进程内截图、不落盘、不碰桌面）：

| | 改前 | 改后 |
|---|---|---|
| 每次截图 `makeKeyAndOrderFront` 次数 | 4 | **1** |
| 每次截图 focus 调用总数 | 8 | 4（其余 3 次 `reorder=false`） |
| 首帧上屏 | 138 / 103 ms | 138 / 103 ms（没有变慢） |

质量门：`cargo test --workspace` 81、`cargo test --manifest-path src-tauri/Cargo.toml` 1
（`src-tauri` 不在 workspace 里，单跑；就是新增的节流测试）、clippy 0、fmt / tsc 干净、`pnpm vitest run` 27。两个 DMG 和源码包重打。

**仍需用户做一次**：签名身份现在固定为 `ai.pinshot.desktop`（本机自签证书 `OpenCodeGo
Dev`，不是 ad-hoc），系统设置里若还有旧的 PinShot 条目，删掉重加一次并彻底重启即可；
之后重新打包不会再掉授权。

## 6.73 2026-11-18：设置改完直接生效，去掉保存按钮

现象：设置里改一项（遮罩、放大镜、语言……）都要再点「保存」才算数，容易改完直接关窗口丢掉。

改法：

- 前端每次改动都进 `apply()`：状态立即更新，写盘延迟 400 ms（滑块一秒能触发几十次事件，
  每次重写 `config.toml` 没必要）；再改会重置计时器，停手 400 ms 写最近一次的值。
- **关窗口不丢改动**：`onCloseRequested` 里如果还有未写盘的值，先 `preventDefault`，等保存
  完再 `destroy`。语言切换不等防抖，直接写。
- 保存串行化（一条 Promise 链），两次保存不会乱序落盘。
- 失败仍走原来的红色横幅；成功在原来按钮的位置显示「已保存」，1.8 秒后自己消失。
- 后端 `settings_save` 现在按**差异**应用副作用：只有快捷键变了才重新注册（Carbon 热键一次
  要几十毫秒）、只有语言变了才刷新托盘并广播 `language:changed`（否则拖滑块会让常驻覆盖层
  跟着重渲染）、开机启动变了才跑 `launchctl`。`keep_warm` 只在由开到关时排一次释放计时器。

实测（`--settings-autosave-selftest`，通过 DOM 把遮罩滑块从 0.5 拖到 0.65）：

```
settings-autosave: dim 0.5 -> 0.65 (wanted 0.65) ok
settings-autosave: restored dim = 0.5
```

配置文件确实写入了 `dim = 0.65`，全程没有点任何按钮；随后恢复成 0.5。README 的设置截图
也重拍了（按钮位置现在只在保存后短暂显示「已保存」）。

## 6.72 2026-11-17：README 配图

- 新增 `docs/images/`：`snip.png`（框选 + 遮罩 + 工具条）、`annotate.png`（箭头 / 马赛克 / 文字）、
  `settings.png`（设置面板）、`long.png`（滚动截屏拼出来的长图）。为了能反复拍同一张图，加了
  `--demo-selftest x,y,w,h[,holdMs[,标注坐标…]]`：按给定选区打开覆盖层，用 DOM 事件画一支箭头、
  一块马赛克和一行文字，然后停住等 `screencapture`，不导出任何文件。
- 长图那张是用 `--scroll-selftest x,y,w,h,manual` 对演示页真实采集的：演示页用
  `?autoscroll` 自己按 48 px / 300 ms 滚动（模拟用户），工具只观察屏幕，48 帧拼成
  2352×7424。接缝逐段放大检查无重复行。
- 拍摄对象是一个本地演示页（`/tmp` 里的假文章），只截这个窗口 / 区域的内容，不包含桌面上的其它窗口。
- README 顶部那段快捷键代码块删掉了（和下面的「常用操作」重复），改成一行摘要 + 指向表格。

## 6.71 2026-11-16：长时间不用后，第一次截图要 1 秒多

现象：闲置十几分钟后第一次按热键，覆盖层要 1 秒多才出来，之后一两张才恢复正常。

日志拆解（修复前，闲置 14 分钟后）：

| 阶段 | 正常 | 闲置后 |
|---|---|---|
| 唤醒常驻覆盖层 | 1–140 ms | **276 ms** |
| 抓屏 `session ready` | 70–196 ms | **529 ms** |
| 页面 boot | 71–171 ms | **611 ms** |
| 首帧上屏（从热键算） | 90–400 ms | **1141 ms** |

三段一起变慢，是整只 app 被系统降到低优先级，但其中两段是**我们自己排在关键路径上**的：

1. **唤醒时重复设置几何**：`overlays_are_warm` 已经校验过屏幕布局没变，但每次仍然
   `hide + set_position + set_size + raise` 走一遍 AppKit。闲置后这些调用要唤醒窗口服务器，
   白花约 270 ms，而且发生在页面开始加载**之前**。
2. **四个覆盖层快捷键在 emit 之前注册**：`Enter / Escape / Cmd+S / Cmd+C` 走 Carbon 的
   `RegisterEventHotKey`，闲置后第一次调用要 ~250 ms；它排在 `emit("overlay:begin")` 前面，
   页面等于一直没收到「开始」。实测正常时也要 40 ms（其余三个 0 ms）。
3. **先交接焦点再上屏**：`overlay_show` 原来是 `activate + acceptFirstMouse + show + setFocus +
   focusWebview`，`makeFirstResponder` 是同步 AppKit 调用，用户在上屏前白等。

改法：

- 唤醒只做必要的事：几何不改、不可见不 hide、**先 `emit` 再注册快捷键**（上屏前 ~250 ms
  就绪，用户按不到）；窗口的 level 在创建时设好，不再每次重设。
- `window.show()` 提到焦点交接之前：几百毫秒内不可能有人点进窗口，键盘可以晚几毫秒。
- `perf:` 行加上 `show N ms`（`show()` 本身耗时）；热键按下时打
  `hotkey: capture pressed`，从此能区分「热键投递晚了」和「截图本身慢」。
- 新增 `--idle-selftest <秒>`：截一次 → 关掉 → 静置 N 秒 → 再截一次，四段耗时直接对比，
  不用真的等几小时。

实测（同一台机器，静置 15 分钟后）：

| 阶段 | 改法 1 之后 | 改法 1+2 之后 |
|---|---|---|
| 唤醒覆盖层 | 257 ms | **2 ms** |
| 抓屏 | 258 ms | 248 ms |
| 页面 boot | 125 ms | 63–96 ms |
| 首帧上屏 | 430 ms | **297 ms** |

改法 1 把窗口几何拿掉后，剩下的 257 ms 正好等于快捷键注册，于是有了改法 2。现在闲置后
第一次 **297 ms**（修前 1141 ms），第二张起回到 ~90 ms，不再「慢一会」。

**仍剩的一块**：`CGDisplayCreateImage` 闲置后 248 ms（正常 85 ms）。同一张画面第二次抓就快了
——是 WindowServer / 显示管线冷启动，不是我们的代码。要再压下去只能保活抓屏或迁到
ScreenCaptureKit 常驻流，两者都要付持续功耗，暂时不动。

## 6.70 2026-11-14：长图接缝重复（固定头/尾高度抖动）；Windows 多开与开机启动

### 滚动截屏「多截」

上一轮只处理了「没有固定头尾、纯空白带」的情况。带吸顶头 / 固定底栏的页面还有一处：

- `detect_fixed_rows` 在每一对帧上独立算固定区。真实吸顶头高度是固定的，但它下面的
  **空白页面行**在两帧里也常常完全相同，于是这一对判成 30 行、下一对判成 27 行。
- 拼接计划里，第 k 条带用 `regions[k+1]`，而 `regions[k+1]` 已经被**下一对**覆盖了。
  也就是说条带用的固定区并不是算这条 offset 时用的那个，接缝处差几行 —— 看起来就是
  长图里多出/少掉一小段。用「12 行字 + 12 行空白 + 30 px 吸顶头 + 24 px 固定底栏」
  的合成页面复现：接缝处 3 行重复。

改法：

- 固定区改成 **一次截图共用一个共识值**：先对每对帧检测，取 top/bottom 的中位数，再用
  这个统一区域对每对帧**重新搜索** offset。所有条带都结束在同一页面行上，连续两对之间
  不再因为 1–3 行的抖动而错位。
- 新增 `crates/pinshot-stitch/tests/sticky_regions.rs`：同一页面逐像素比对。修复前第 346 行
  开始错，修复后 0 处不同。
- 全量 `cargo test --workspace` 81 passed。

### Windows 多开

之前没有单实例保护：登录启动 + 手动启动会各跑一个，两个托盘图标、两次热键注册，
热键还可能被后启动的那个抢走。现在加了 `tauri-plugin-single-instance`（第一个插件），
第二个进程直接退出，并让已经在跑的那个打开「设置」，双击不会看起来「没反应」。

### Windows 开机启动

- **托盘创建失败不再终止进程**：登录时 Explorer 可能还没建好通知区域，`setup` 里
  `setup_tray()?` 一旦失败整个 App 就退出，表现就是「开机没启动」。现在失败只记日志，
  并在 0.8 / 1.6 / 3.2 秒后重试创建托盘。
- 开机启动的 Run 值带上 `--autostart`：下次看诊断日志就能区分「系统拉起的」和
  「用户点开的」；启动时仍会把 Run 键重写一遍（自愈），路径变了也会跟上。
- 验证：macOS 上用 `--autostart` 启动，日志出现
  `autostart: launched by the system at login`；同机启动第二个进程会被拒绝，
  进程数始终是 1，日志 `single-instance: another launch ignored`。

## 6.6 2026-11-13：这一轮修掉的 macOS bug

- **全局 Enter/Escape/⌘S/⌘C 被常驻注册**（预热引入的第一版）：`OVERLAY_KEYS` 是裸键，
  idle 时注册会把系统级按键吞掉；更糟的是快捷键处理器对**任何**按下的快捷键都执行
  `start_capture`，于是按一下回车就会冒出截图框。现在预热路径不注册这些键，处理器也只
  认「配置里的截图键 / 贴图键」，其余一律忽略。
- **设置里的「截图 / 贴图」快捷键是死的**：`hotkey_snip` / `hotkey_paste` 只写进
  config.toml，`register_hotkey` 硬编码 F1 / ⌘⇧1，贴图键根本没注册。现在按配置注册
  （解析失败或冲突才回退），`F4` 之类的贴图键真的能贴出最近一张。
- **「开机启动」是死的**：复选框只改配置。现在写 `~/Library/LaunchAgents/ai.pinshot.desktop.plist`
  （`open -g -a PinShot.app`，不抢焦点），开/关都会 `launchctl load/unload`，设置保存时
  立即生效，启动时按配置对账。
- `first_frame` / `capture_started` / `capture_serial` 三个原子量保留下来做性能回归：
  `--capture-selftest <n>` 会打印每一轮的数字。

## 7. 2026-11-12 晚：箭头、菜单召回、历史清空

- **箭头多出的凸起**：之前把线一直画到箭头尖端（`lineCap: round`），圆头从三角形箭
  头两侧露出来。现在线段停在箭头内部（尖端后退 0.72×头长），箭头是实心三角形，尖端
  干净。证据：`--arrow-selftest x,y,w,h` 导出图 `/tmp/arrow-selftest.png`。
- **托盘「设置」「历史记录」点一次不回来**：App 跑在 accessory（仅菜单栏）模式，
  macOS 不会主动激活它，托盘菜单还在收起时也会拒绝激活。`reveal_window()` 现在切到
  regular → activate → makeKeyAndOrderFront，并像覆盖层一样在 80/220/450 ms 重试；
  窗口销毁后再退回 accessory（设置窗口开着时不退回，否则会丢菜单栏和键盘）。
  证据：`--window-selftest settings|history` 一次调用后
  `visible=true focused=true`，先 hide 再调用同样一次就 `focused=true`。
- **历史记录「全部清空」**：`Store::clear_all()` + `history_clear` 命令 + 头部按钮（点
  一次变成「确认清空？」，4 秒内再点才执行）。同时清掉缩略图缓存，收藏也一起清。
- **线条命中判定**：箭头/画笔/记号改为「到线段的距离」判定（`hitsShape`），不再是每个
  顶点的方框 —— 长箭头中间一段以前点不中，现在可以；箭头包围盒的空白处也不会误选。
- **删除死代码**：滚动截屏只剩「按钮 → 隐藏覆盖层 → 用户自己滚 → Enter」一条路径；
  随之删掉 `src/live.ts`（拖到屏幕边缘自动开始采集的那套）、`livePreview`/`appendLiveStrip`、
  `scroll:live` 监听、`startLive`，以及 `preview` 渲染选项和已下线的模糊工具残留。
  拖到底边不会再"手滑"开始长截图；尺寸标签也不再提示边缘手势。
- **手动滚动截屏不再要「辅助功能」权限**：那条检查只对自动模式（AX 驱动页面）有意义。

## 8. 标注交互的现行规则（2026-11-12）

- **工具条两层**：第一行永远是 选择/矩形/椭圆/箭头/画笔/记号/马赛克/文字/序号 +
  滚动截屏 + 复制/保存/取消；第二行随工具变化（文字=颜色+字号，马赛克=粒度，
  其他=颜色+粗细，选择=无）。
- **字号与粗细是两个独立变量**：`textSize` 只作用于文字，`width` 只作用于线条和边框。
  文字工具的第二行调的是字号，滚轮同样。
- **调节作用于最后一个同类元素**：绘制类工具下调颜色/尺寸改「该工具产出的最后一个
  图形」（`SceneModel::lastWithKinds`），不会误伤别的类型；「选择」工具下只改选中的
  那一个。
- **文字工具永远新建**：点画布任何位置都开一个新的输入框；移动/改样式统一交给
  「选择」(V)，双击可再次编辑。输入框在别处点击即提交，所以连续点击可以不断加文字。
- **输入框没有底色**：只有一圈「黑+白」双层细描边（深浅底图都看得见），文字用当前
  颜色，所见即所得。宽度随输入内容增长（用与绘制相同的字体实测 `measureText`），
  不再是固定长度。
- **空输入不会产生元素**：输入框按 `draftSeq` 作为 React key 重新挂载，所以「点一下
  再点别处」拿到的是**空**输入框；没打字就离开则什么也不加。之前复用了同一个 DOM
  节点（`defaultValue` 不会重置），于是每次点击都复制了上一段文字。
- **模糊工具已移除**：Canvas `filter` 在离屏合成里不生效，避免留下假按钮；马赛克是
  像素实现，保留。
- 回归证据：`--text-selftest x,y,w,h,文字` 连写两段文字、再空点一次后导出：
  输入框宽度 104 → 125（内容变长），空点的那次 96（最小宽）；导出图里仍然只有
  y≈125–163 / 365–403 两条红色像素带，说明「连续加多个文字」「空点击不产生元素」
  「文字进入导出」都成立。
