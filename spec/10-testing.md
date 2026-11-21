# 10 · 测试策略

## 1. 测试金字塔

```
        ┌──────────────────────┐
        │ 手工验收（3 平台）    │  ← 每里程碑一次，清单见 §5
        ├──────────────────────┤
        │ 端到端（Playwright）  │  ← 覆盖层交互、贴图窗口操作
        ├──────────────────────┤
        │ 集成测试（crate 间）  │  ← 抓帧→拼接→存盘→历史
        ├──────────────────────┤
        │ 单元测试              │  ← 几何、拼接、场景图、剪贴板解析
        └──────────────────────┘
```

原则：

- **能用纯函数测的，绝不放到 GUI 里测。** 拼接、场景图、剪贴板解析、保留策略都是纯逻辑，
  必须在 CI 里毫秒级跑完。
- **屏幕相关的部分用「帧序列」替代。** 任何需要真屏幕的测试都能被一段帧序列 + 一个
  `CaptureBackend` 的 fake 实现替代。
- **GUI 测试只验证"连起来了"**，不验证像素（像素归视觉回归）。

## 2. 已实现（`crates/pinshot-stitch`）

10 个测试，0.2 s：

| 测试 | 覆盖 |
| --- | --- |
| `stitches_a_long_page_with_a_sticky_header` | 粘性页眉只出现一次；位移精确；重复帧触发停止 |
| `a_fixed_sidebar_does_not_bias_the_offset` | 固定侧栏不污染位移估计 |
| `a_sticky_footer_is_dropped` | 吸底工具栏不进长图 |
| `survives_brightness_drift_and_sensor_noise` | ±8% 亮度漂移 + σ≈3 噪声 |
| `stops_when_the_page_stops_moving` | 到底检测 |
| `refuses_frames_it_cannot_align` | 无关联帧返回 `LowConfidence` 而不是坏图 |
| `truncates_at_the_height_limit` | 高度保护 |
| `rejects_frames_of_different_sizes` | 输入校验 |
| `converts_rgba_to_luma` / `copy_rows_moves_a_band` | 基础图像操作 |

## 3. 待建测试矩阵

| 层 | 范围 | 工具 |
| --- | --- | --- |
| 单元 | `pinshot-core` 几何/DPI、`annotate` 场景与撤销、`store` 保留策略、剪贴板解析 | `cargo test` |
| 集成 | fake backend + 帧序列 → 拼接 → blob → 历史查询 | `cargo test` + `tempfile` |
| 属性测试 | 随机页面/滚动序列，断言「拼出高度 = 页面高度」；随机几何变换的撤销可逆性 | `proptest` |
| 视觉回归 | 5 张基准图 × 每种标注工具的导出结果 | Playwright + 像素差异阈值 0.2% |
| E2E | 热键 → 覆盖层 → 拖框 → 标注 → 复制；贴图窗口的关闭/恢复/销毁 | Playwright（Tauri MCP） |
| 手工 | 三平台权限、混合 DPI、多屏、真实站点滚动 | 清单 §5 |

## 4. 滚动截屏基准集

20 个页面，录制为帧序列（`fixtures/scroll/<name>/frame-000.png` + `meta.json`）：

| 类别 | 例子 | 要验证什么 |
| --- | --- | --- |
| 长文档 | GitHub issue、PR diff | 基本拼接、代码块不被切 |
| 阅读页 | 知乎回答、微信公众号 | 粘性页头、图片懒加载 |
| 编辑器 | Notion 文档 | 固定侧栏、悬浮工具栏 |
| 密集表格 | 后台列表（1000 行） | 重复行不误匹配 |
| 无限滚动 | 社交信息流 | max_height / max_frames |
| 深色模式 | 任意 | 亮度归一化 |
| 含视频 | 新闻页 | 动画区域投票 |
| 高 DPI | 2x 屏录制的同一页 | 缩放比处理 |
| 横向滚动 | 宽表格 | 2D 拼接（P2） |
| 遮挡 | 另一个窗口盖住一半 | 窗口级捕获 |

`meta.json` 记录：期望总高度、关键行（页面中某段文字的 y 坐标）、滚动步长、是否应降级。

跑分命令：

```bash
pinshot bench scroll --frames fixtures/scroll --json
```

输出每个页面的：是否成功、位移误差、耗时、长图高度偏差、接缝数。CI 里只跑其中 5 个
（体积小），全量在里程碑门禁时跑。

## 5. 手工验收清单（每里程碑跑一次）

**基础**

- [ ] 三屏 + 混合 DPI：覆盖层正确，选区跨屏导出无错位
- [ ] 热键在所有常见 app 前台时都能唤起（含全屏应用、游戏、远程桌面）
- [ ] 权限被拒 → 引导可恢复；权限被撤销后重新检测能发现
- [ ] `Esc` 在每一层都只退一层

**截图**

- [ ] 截图后前台 app 仍是原 app（焦点没丢）
- [ ] 剪贴板里的图粘贴到 备忘录 / Slack / Figma，尺寸与色彩一致
- [ ] 放大镜像素与真实像素一致（对着一张 1px 网格图验证）

**标注**

- [ ] 每种工具画一遍，导出的图与所见一致
- [ ] 100 次撤销/重做不崩、不卡
- [ ] 马赛克后原图信息不可恢复

**贴图**

- [ ] 关闭 → 恢复；销毁 → 不可恢复；隐藏全部 → 全部回来
- [ ] 退出应用后，上次"隐藏"的贴图恢复，"关闭"的被销毁
- [ ] 20 张 4K 贴图，拖动不掉帧

**滚动**

- [ ] 20 个基准页面全部跑一遍，记录失败原因
- [ ] 人为把窗口盖上去，验证窗口级捕获
- [ ] 滚太快（一次滚 2 屏）→ 给出 `NoOverlap` 提示而不是坏图

## 6. CI

```yaml
# 每个 PR
- cargo fmt --all --check
- cargo clippy --workspace --all-targets -- -D warnings
- cargo test --workspace
- pnpm typecheck && pnpm build
- pnpm e2e            # 只跑核心 3 条流程

# 每夜
- pnpm e2e --all
- pinshot bench scroll --frames fixtures/scroll/smoke
- 视觉回归全量
```

GUI 的 E2E 在三平台矩阵上跑（GitHub Actions 的 macos / windows / ubuntu runner）；
Linux 上受 Wayland 限制，只跑 Xvfb + X11。

## 7. 缺陷分级

| 级别 | 定义 | 例子 |
| --- | --- | --- |
| P0 | 数据丢失 / 崩溃 / 产出错误内容 | 拼接错位却未提示；历史记录指向空文件 |
| P1 | 主流程受阻但有绕路 | 自动滚动失败未降级 |
| P2 | 体验问题 | 工具条位置偶尔遮挡选区 |
| P3 | 外观 | 圆角 1px 差异 |

P0 必须当天修；发布前 P0/P1 清零。
