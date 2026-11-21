# 历史设计稿

这里的 00–09 是立项时的设计与计划（定位、需求编号、crate 划分、抓屏/滚动/标注/贴图/
历史/性能的初版方案）。**它们描述的是「当时打算怎么做」，不是现在的实现**：

- 现状、已修掉的问题、实测数字：看 [`../docs/STATUS.md`](../docs/STATUS.md)
- 使用者文档（功能、快捷键、配置、构建）：看 [`../README.md`](../README.md)
- Windows 现状：看 [`../docs/WINDOWS.md`](../docs/WINDOWS.md)

已经被实现改掉、但保留在这里当决策记录的例子：

- 滚动截屏从「拖到边缘自动注入滚轮」改成了「点按钮 → 用户自己滚 → Enter」，
  `04-scrolling-capture.md` 里的自动采集路径界面已不再使用（库里的 `auto_scroll`
  仅剩 CLI / 自测在用）。
- 标注里的局部模糊工具已删除（WebView Canvas `filter` 在离屏合成里不可靠），
  改用马赛克。
- 截图覆盖层不再在每次截图时切到「普通 App」，改为 accessory + 提前激活，
  见 `docs/STATUS.md` 6.56。
