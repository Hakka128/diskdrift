# WhyBig — Milestone 4 设计文档 (since / retention / snapshot-json / migration)

> 目标：让 WhyBig 适合长期真实使用、接近 v0.1.0 首次公开。
> 原则：**记录、比较、解释；不清理用户电脑、不偷偷联网、不偷偷后台运行。**

---

## 0. 阶段边界

实现：`--since`、`whybig prune`(retention)、`snapshot --json`、migration 硬化(v2 meta 表)、`whybig compact`(显式 VACUUM)、README v0.1.0 版 + tools/demo、CI release smoke。
不实现：文件级历史、GUI/TUI/daemon/watcher、删除任意用户文件、anomaly、cloud、telemetry、AI、软件专用规则。

## 1. `--since` 选择语义（UTC，确定性）

- 解析：`m|h|d|w` 后缀 + 正整数；`0`/非法/溢出 → CLI 参数错误。不做 month/year/自然语言/日期串。
- `target_time_ms = latest_created_at_ms - duration_secs*1000`（整数 UTC，无本地 TZ）。
- before 选择：`created_at <= target_time` 中 `created_at` 最大者；created_at 并列时取 `id` 最小（确定性）。
- 若该 root 无 `<= target_time` 的快照：用该 root **最早**快照，并设置 `used_earliest=true`。
- Domain 暴露（DiffSelection::Since 解析后随 diff/top 一起返回）：
  `SinceInfo { requested_seconds: i64, requested_target_ms: i64, effective_before_ms: i64, used_earliest: bool }`
- Human 在最前面提示（used_earliest 时）：
  ```
  Requested: 30 days ago
  Available history starts: Sep 5
  Using earliest available snapshot.
  ```
- JSON 增加 `since: {requested_seconds, requested_target, created_at_rfc3339, effective_before, used_earliest}`（仅 --since 时出现）。
- `--since` 与 `--from/--to` 由 clap `conflicts_with` 在解析期拒绝（exit 2）。
- Storage 新增 `get_snapshot_at_or_before(root, time_ms)`（`ORDER BY created_at DESC, id ASC LIMIT 1`）与 `earliest_snapshot_for_root(root)`。

## 2. Retention 算法（UTC bucket，plan/executor 分离）

```
struct RetentionPolicy { recent_days, daily_days, weekly_days }  默认 7/30/365
bucket_key(kind, utc_ms):
  daily  -> epoch_day (utc_ms / 86400_000)
  weekly -> epoch_day / 7          // 明确 7-day bucket，非 ISO week
age = now_utc_ms - created_at_ms
规则（每 tracked root 独立）：
  - 最新 snapshot 永远保留；root 只有 1 条 → 保留。
  - age <= recent_days*day  -> 全部保留
  - recent_days*day < age <= daily_days*day -> 每 daily bucket 保留 1（取 bucket 内 created_at 最大，并列取 id 最大）
  - daily_days*day < age <= weekly_days*day -> 每 weekly bucket 保留 1（同上规则）
  - age > weekly_days*day   -> 删除
```
- **UTC**：daily bucket=epoch_day，weekly=epoch_day/7；避免 local TZ/ISO week 边界误差；year boundary / 12-31 / 01-01 / week-53 语义退化到纯 epoch day，天然无 ISO 复杂。
- `RetentionPlanner`（纯函数，可测）: `build_plan(policy, snapshots_by_root, now_ms) -> PrunePlan`
  `PrunePlan { keep_ids, remove_ids, roots: [{root, keep, remove}] }`（prune 预览与 apply 用同一 plan）。
- `PruneExecutor`：**唯一**能改库的组件；单 `BEGIN IMMEDIATE` 事务 `DELETE FROM snapshots WHERE id IN(...)`，FK ON ⇒ entries 级联；任一步失败 ROLLBACK，原库不动。**绝不碰用户文件。**
- `now` 作为显式参数传入 planner（测试可注入固定时间；生产用 `Utc::now()`）。

## 3. `whybig prune` CLI

- 默认 dry-run：打印预览（snapshots 总数/将保留(分级计数)/将删除/最早被删日期/DB 大小 + 明确 "No data has been deleted." + "Run `whybig prune --apply` to apply this retention policy."）。
- `--apply` 才执行删除；无 `-y/--force` 绕行。
- `--json`：默认 dry-run 也输出 JSON（`applied:false`）；`--apply --json` 后 `applied:true`。stdout 仅 JSON。
- dry-run 与 apply 必须基于**同一** plan（先 build_plan，再决定 dry/apply）。
- 删除后提示 SQLite 保留空闲页（文件未必立即缩小）；不自动 VACUUM。

## 4. `snapshot --json`

- JSON 模式：无进度条、无 "Scanning"、stdout 仅一段 JSON；fatal 错误仍走 stderr+exit 1。
- warnings 进入 JSON（`warnings:[{path,error,kind}]`），不再打 stderr。
- `SnapshotOutcome` 增加 `warnings` 字段（复制自 ScanResult），Human 输出行为不变。
- Schema（JsonSnapshotReportV1）：snapshot_id/created_at(root RFC3339 UTC)/root/size_kind/total_size_bytes/file_count/dir_count/scan_duration_ms/skipped_count/warnings/schema_version=1/command="snapshot"。

## 5. Migration 硬化

- `MIGRATIONS` 增 `v2`: `CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);`（真实用途 + 迁移测试载体）。
- 打开时若 `user_version > 当前最大` → `WhyBigError::NewerSchema`，消息 "Database was created by a newer version of WhyBig. Please update WhyBig."，拒绝任何写。
- 每次迁移在**独立事务**内（execute_batch 主体 + user_version 递增同事务）；失败回滚，原 DB 不破坏（既有 `migrate()` 已有此结构，补未来版本守卫）。
- 测试：真造 v1 库(手写 v1 建表+插数据)→当前代码打开→升到 v2 且数据保留、meta 表存在；重复打开幂等；user_version=999 拒绝。

## 6. `whybig compact`

- 显式命令，仅 `VACUUM`（清理空闲页后文件缩小）；提示需临时磁盘空间（≈库大小）；可选 `--json`（延期，M4 不强制）。
- 不自动执行。

## 7. CLI/README/发布

- 帮助文案/参数命名抽查；exit code 沿用 clap(usage=2，运行失败=1，成功=0)。
- README v0.1.0：定位语 → demo(真实 binary 输出) → Problem → Quick Start → Commands → How it works → **Privacy**(本地/无遥测/无网络/无云/不读文件内容/只存目录级 size/不删用户文件) → Performance → Limitations → Installation → Roadmap。
- `tools/demo`：确定性脚手架，驱动真实 binary 生成 README 场景。（实现为 `examples/demo.rs` + 使用文档）
- CI：加 release 构建冒烟 job（build --release；--help；init temp；snapshot temp；diff）。

## 8. 测试规划（≈50 新增）

- since(12)：normal/恰好命中/范围前无快照/多点附近/多 root/与 from-to 冲突(clap)/非法时长/0 时长/超大时长/时间并列确定性/scoped top/JSON requested effective。
- retention(16)：recent 全留/daily/weekly/>365 删/最新必留/单条 root/多 root 隔离/bucket 取最新/dry 不改库/apply 删+级联/事务回滚/重复 apply 幂等/跨年/跨周边界/未来时间戳/policy 校验/prune JSON。
- snapshot-json(5)：合法 JSON/stdout 纯净/skipped warnings/Unicode root/JSON 无进度。
- migration(5)：旧库升级/数据保留/重复迁移/失败回滚/未来版本拒绝。
- e2e(8)：历史快照→diff --since / top --since / prune preview / prune apply / prune 后 history / snapshot --json / tools-demo smoke。
