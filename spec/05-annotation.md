# 05 · 标注

## 1. 模型：场景图，不是像素

标注不是画在底图上的像素，而是一串可重放的操作：

```rust
pub struct Scene {
    pub shapes: Vec<Shape>,
    pub revision: u64,
}

pub struct Shape {
    pub id: ShapeId,
    pub kind: ShapeKind,
    pub style: Style,
    pub transform: Transform,   // 旋转/平移，物理像素坐标系
    pub z: u32,
}

pub enum ShapeKind {
    Rect { rect: Rect, corner: f32 },
    Ellipse { rect: Rect },
    Line { points: Vec<Point>, arrow: ArrowStyle },
    Pen { path: Vec<Point>, width: f32 },
    Marker { path: Vec<Point>, width: f32, alpha: u8 },
    Mosaic { region: Region, block: u16 },
    Blur { region: Region, radius: f32 },
    Text { at: Point, content: String, size: f32, angle: f32, stroke: Option<Stroke> },
    Counter { at: Point, number: u32, radius: f32 },
}

pub struct Style {
    pub color: Color,       // RGBA，8 bit/通道
    pub width: f32,
    pub fill: Option<Color>,
    pub dash: Option<Dash>,
}
```

为什么是场景图：

- **撤销/重做**只是移动 `shapes` 的指针，不需要保存位图快照（一张 5K 截图快照 59 MB，
  100 步就是 6 GB）；
- **导出**时才光栅化，因此 1x / 2x 导出可以各画一次，边缘永远锐利；
  （实现：场景与光栅化都在前端 `src/scene.ts` / `src/render.ts`，导出把同一份绘制
  结果编码成 PNG 交给 Rust —— 见 `spec/02` ADR-4 的修正说明）
- **二次编辑**（P1）天然可行：改一个 `Shape` 就够，不用重画；
- **长图**：滚动截屏得到 2 万像素的底图时，标注仍然只是几条几何数据。

代价：马赛克和模糊需要底图像素，不能纯几何。这两类在**应用时**生成一次结果位图，
作为 `ShapeKind::Mosaic { block_source: BlobId }` 缓存，撤销时丢弃缓存。

## 2. 工具与交互

| 工具 | 键 | 拖拽语义 | 修饰键 |
| --- | --- | --- | --- |
| 矩形 | `R` | 对角拖动 | `Shift` 正方形 |
| 椭圆 | `O` | 对角拖动 | `Shift` 正圆 |
| 直线 | `L` | 起点 → 终点 | `Shift` 45° 吸附；`Tab` 切换箭头 |
| 折线 | `L`（再按） | 逐点单击，双击/右击结束 | — |
| 箭头 | `A` | 同直线 | `Shift` 45° |
| 画笔 | `P` | 自由绘制 | — |
| 记号笔 | `H` | 自由绘制，半透明加粗 | — |
| 马赛克 | `M` | 框选区域 | 滚轮改块大小（8/16/32） |
| 模糊 | `B` | 框选区域 | 滚轮改半径 |
| 文字 | `T` | 单击进入内联编辑 | `Shift` 拖角保持水平 |
| 序号 | `N` | 单击递增 | `Shift` 重置为 1 |
| 橡皮 | `E` | 单击/拖过删除命中的形状 | — |

通用：

- `Ctrl+Z` / `Ctrl+Shift+Z` 撤销重做；`1` `2` 调线宽；滚轮调线宽；`[` `]` 微调。
- 右击结束当前图形；右击（无图形进行中）取消当前工具，回到选区态。
- `空格` 显示/隐藏工具条（给"我要看全图"的人）。
- `Esc` 退出标注（若已有图形，第一次 Esc 取消当前图形，第二次退出）。
- 所有工具记住上次的颜色与宽度，跨会话持久化。

## 3. 渲染管线

```
底图（冻帧或长图）───► base canvas（一次性 putImageData / ImageBitmap）
                          │
标注场景 ──► 脏矩形计算 ──► overlay canvas（requestAnimationFrame）
                          │
交互元素（选中框、把手）──► interaction canvas（DOM 层，pointer-events: none 的兄弟节点）
```

- **脏矩形**：一次笔画只重绘受影响的行条带（`dirty: Rect`），5K 全屏下笔迹延迟 < 3 ms。
- 预览（正在拖的图形）画在 overlay 的临时层，松开时才提交到 `Scene`。
- 导出：新建一个离屏 canvas，按 `export_scale` 画底图 + 场景，一次 `toBlob`。
- 文字渲染：canvas `fillText` + `strokeText`（描边在外层），字体 `system-ui`，
  中文回落到 PingFang / Microsoft YaHei；字号按物理像素。

## 4. 马赛克与模糊

- **马赛克**：块平均，块大小 8/16/32 物理像素；实现为整数块的 box filter，
  在 Rust 侧用 rayon 并行（14 MP 图约 6 ms）。
- **模糊**：`box blur ×3` 近似高斯（半径 r 的三次 box 半径约 `r/2`），
  边界用 edge clamp；比真高斯快 5 倍，视觉无差别。
- 两者都在**底图像素**上算，结果缓存为 blob；场景只引用 blob id。
- 安全承诺：马赛克区域在导出图中**原始像素不存在**（不是"盖一层半透明"）。
  这条要在测试里断言（把马赛克区域的导出像素与原始像素做比较，必须不同且方差极小）。

## 5. 撤销栈

```rust
enum Command {
    Add(Shape),
    Remove(Shape),
    Modify { id: ShapeId, before: Shape, after: Shape },
    Clear(Vec<Shape>),
}
```

- 栈上限 200 条；超过时丢弃最旧的一条（不是最旧的 1/2，避免一次卡顿）。
- 每次提交带一个 `revision`，供前端增量同步（只传 `Command`，不传整个场景）。
- 撤销后重做栈保留，任何新操作清空重做栈（标准语义）。

## 6. 命中测试

- 矩形/椭圆：边界带 ±6 px 容差（缩放手感），内部命中仅在填充时生效；
- 线段：点到线段距离 ≤ `width/2 + 4`；
- 文字：旋转后的 OBB 检测；
- 从 `z` 大的往下找，第一个命中即选中；
- 多选（P2）：`Shift` 点选 + 框选。

## 7. 导出

| 方式 | 键 | 行为 |
| --- | --- | --- |
| 复制到剪贴板 | `Enter` / `Ctrl+C` / 双击选区内 | PNG（保留 alpha），同时写文件到剪贴板（可选） |
| 保存到文件 | `Ctrl+S` | 弹保存面板，默认名 `PinShot_2026-11-11_14-32-05.png` |
| 快捷保存 | `Ctrl+Shift+S` | 直接进 `quick_save_dir`，不弹窗，toast 提示路径 |
| 贴到屏幕 | `Ctrl+T` / 中键单击选区 | 创建贴图窗口 |
| 仅标记成功 | `-o success` | 进历史，不产出 |

编码：PNG（默认，无损）/ WebP（可选，质量 -1 自动）/ JPG（仅在不含 alpha 时可选）。
`Ctrl+Shift+C` 在已有文字形状时复制「文字内容」而不是图片。

## 8. 与底图缩放的关系

- 场景坐标永远是**物理像素**；
- 导出 2x 时，几何量乘 2，线宽乘 2，字号乘 2，马赛克块乘 2 —— 视觉结果一致；
- 不允许「先导出 1x 再放大」，那会让文字发虚。

## 9. 测试

- 场景图单测：每个 `ShapeKind` 的光栅化结果在固定输入下的哈希稳定（防止无意改动渲染）。
- 撤销栈：随机 200 步操作后回退到底，场景必须与初始逐字段相等。
- 马赛克安全：断言原始像素不可从导出图恢复（块内方差 < 1）。
- 视觉回归：5 张基准图的截图对比（阈值 0.2% 像素差异）。
