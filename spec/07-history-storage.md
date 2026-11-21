# 07 · 历史与存储

## 1. 布局

```
~/Library/Application Support/ai.pinshot.desktop/
├── config.toml
├── pinshot.db                # SQLite（WAL）
├── pinshot.db-wal
├── blobs/
│   ├── 3f/3f9a…c1.png        # 内容寻址：blobs/<前 2 位>/<blake3>.<ext>
│   └── 7b/…
├── thumbs/
│   └── 3f/3f9a…c1.png        # 长边 ≤ 320，导出失败时 UI 用它兜底
└── logs/

~/Library/Caches/ai.pinshot.desktop/
└── scroll/<job-id>/frames/   # 滚动任务的原始帧，任务成功后按配置清理
```

Windows：`%LOCALAPPDATA%\ai.pinshot.desktop\`；Linux：`~/.local/share/ai.pinshot.desktop/`。

## 2. 为什么 blob 不进 SQLite

- 一张 5K PNG 约 3–8 MB；10 万张就是几百 GB。SQLite 能放，但 `VACUUM` 和备份会变成灾难。
- 内容寻址带来免费去重：同一张图贴 5 次、截图与贴图共用一份字节。
- 用户可以手动备份/迁移图片，而数据库只存索引。
- 代价：**必须维护一致性**。写入顺序固定为
  `写临时文件 → fsync → rename 到最终路径 → 事务写索引`。
  索引里有而文件没有 = 显示占位图；文件有而索引没有 = 下次 GC 删除。

## 3. 表结构

唯一来源是 [`schema/001_init.sql`](schema/001_init.sql)，Rust 侧用 `include_str!` 编译进
二进制，**不允许在代码里另写 DDL**。

核心表：

| 表 | 作用 |
| --- | --- |
| `captures` | 一次成功截图（含滚动合成图）的元数据 |
| `capture_frames` | 滚动任务的原始帧与每帧位移，用于复现与"保留原始帧" |
| `scroll_jobs` | 一次滚动任务的模式、状态、置信度、接缝 |
| `pins` / `pin_groups` | 贴图窗口的位置、变换、状态（可恢复语义） |
| `settings` | 运行时设置镜像（`config.toml` 优先） |
| `ocr_text` + `ocr_fts` | P2 的文字识别结果与全文索引 |

索引设计对应三条真实查询：

1. 历史面板首页：`captures_recent (created_at DESC)`；
2. 保留策略清理：`captures_cleanup (favorite, created_at)` —— 先按收藏过滤，再按时间删；
3. 按应用筛选：`captures_app (app_name, created_at DESC)`。

## 4. 写入路径

```
截图成功
  → 编码 PNG（writer 线程）
  → 写 blob（临时文件 → fsync → rename）
  → 写 thumb
  → BEGIN; INSERT captures; INSERT capture_frames; COMMIT;
  → 通知 UI（capture_id）
```

任何一步失败都不留下"半条记录"：blob 先落地、索引后提交，因此崩溃只会产生
**孤儿文件**（GC 清理），不会产生**指向空文件的记录**。

## 5. 保留策略

```toml
[history]
keep_days = 30
keep_items = 1000
max_size_mb = 2048
```

启动时与每 6 小时各跑一次清理：

```sql
-- 1) 超天数 / 超条数（收藏者除外）
SELECT id FROM captures
WHERE favorite = 0 AND deleted_at IS NULL
ORDER BY created_at ASC
LIMIT (SELECT MAX(0, COUNT(*) - :keep_items) FROM captures WHERE favorite = 0);

-- 2) 超容量：按时间从旧到新累加，直到总量 <= max_size_mb
```

删除 = 先删索引行（同一事务），再删 blob。若 blob 被其他 capture 引用
（内容寻址下完全可能），`blobs` 里还有引用计数 —— 用一次 `SELECT COUNT(*) FROM captures
WHERE blob_hash = ?` 判断即可，不额外建表。

## 6. 迁移

`PRAGMA user_version` 驱动：

```rust
match version {
    0 => apply(include_str!("schema/001_init.sql")),
    1 => { /* 1 → 2 */ }
    v => return Err(unsupported(v)),
}
```

- 迁移必须在**一个事务**里完成，失败则整体回滚并提示"备份后重试"。
- 破坏性变更（删列）走"建新表 → 拷贝 → 改名"，SQLite 的 `ALTER TABLE` 能力有限。
- 迁移前自动复制一份 `pinshot.db` 到 `backups/pinshot-<version>-<ts>.db`（保留 3 份）。

## 7. 隐私

- 默认**不写**任何 EXIF、作者、设备名。PNG 只保留必要块。
- 历史全在本地，没有任何上传路径；`doctor` 命令可以断言"无网络模块被初始化"。
- 删除历史 = 真的删文件（不是移到废纸篓）；提供一次"彻底清除"（含 WAL 与 thumbs）。
- 默认不记录窗口标题？记录。窗口标题常常含文档名，属于敏感信息，因此：
  - 设置里提供「不记录窗口标题」（只存 app 名）；
  - 默认开启记录，因为历史面板靠它检索。

## 8. 一致性检查（`pinshot doctor --check-storage`）

1. `captures.blob_hash` 指向的文件存在且 blake3 校验通过；
2. `blobs/` 中的孤儿文件（无索引引用）列表；
3. `thumbs/` 缺失但 blob 存在的条目 → 重新生成；
4. WAL 大小 > 64 MB → 提示 `wal_checkpoint(TRUNCATE)`；
5. `PRAGMA integrity_check`。

输出可机读 JSON，供 CI 与用户反馈使用。
