# WhyBig — Milestone 3 设计文档 (history / top / JSON / benchmark)

> 定位不变：WhyBig 是 Disk Growth Debugger。本阶段新增"过去怎么变化 / 最近谁涨最多 / 机器可读输出 / 可复现性能基线"。

---

## 0. 阶段边界

实现 `whybig history`、`whybig top`、diff/inspect/history/top/status 的 `--json`、benchmark framework（确定性生成器 + runner + BENCHMARKS.md）。
不实现：文件级历史、GUI/TUI/daemon/watcher/后台/自动清理/删除/anomaly/云同步/telemetry/AI/软件专用规则。
retention **只做设计不实现**（§9）；**本阶段不允许任何自动数据删除**。

---

## 1. history data model

```rust
// src/history/
pub struct HistoryPoint {
    pub snapshot_id: i64,
    pub created_at_ms: i64,          // 稳定排序键（与 id 单调一致）
    pub size: u64,
    pub delta_from_previous: Option<i128>,  // 首点为 None
}
pub struct HistoryReport {
    pub root: PathBuf,
    pub target: PathBuf,             // root 自身或 <path> 归一化结果
    pub points: Vec<HistoryPoint>,   // 时间升序
    pub total_delta: i128,           // last.size - first.size
}
```
- 默认 root = 最新 snapshot 的 root（与 diff 选择逻辑一致 —— 直接读 `storage.latest_snapshot()`）。
- 缺失目录 = 0（LEFT JOIN COALESCE），不做"丢掉时间点"。

## 2. top 是否直接复用 diff engine —— 是（零复制逻辑）

`top` 不碰 SQL/不重新发明 delta：
```
select_pair(Default)              // 复用 diff::select_pair
scope = root 或 normalize_target(<path>)（within-root 校验同 inspect）
d = diff::compute(storage, before, after, &scope)   // 复用 M2 attribution
Growth → d.grew      Shrink(`--shrink`) → d.shrank
```
`top <path>` = scope=path 的 grew，与 `inspect <path>`.contributors 同源（差异仅展示）。
`TopReport { root, scope, before, after, mode, entries }`。

## 3. JSON public schema（独立 API structs，非内部 struct 直出）

独立模块 `src/json/`，struct `Json*ReportV1`（serde derive，字段全部自定）。
- 顶层必有 `"schema_version": 1` + `"command"`。
- 时间统一 **RFC3339 UTC**（消除 TZ 歧义；human 渲染仍本地时间）。
- 所有 size/delta 一律**整数 bytes**，不输出 "11.8 GB"。
- Added/Removed/Changed → `"added"/"removed"/"changed"`（serde snake_case）。
- JSON 模式：stdout 仅一段合法 JSON；进度/诊断走 stderr 或字段。

核心结构（与 spec 示例对齐，加必要字段）：

```json
{"schema_version":1,"command":"diff","root":"...","before":{"snapshot_id":1,"created_at":"Rfc3339","size_bytes":123},"after":{...},"total_before_bytes":N,"total_after_bytes":N,"delta_bytes":333,"grew":[{"path":"...","before_size_bytes":..,"after_size_bytes":..,"delta_bytes":..,"state":"changed"}],"shrank":[]}
```
inspect 加 `path/target_before_bytes/target_after_bytes/target_delta_bytes/contributors/other_bytes`；
history 加 `path/points[{snapshot_id,created_at,size_bytes,delta_from_previous_bytes:null|int}]/total_delta_bytes`；
top 加 `path/mode/entries`；
status 加 `initialized/data_dir/database_path/database_size_bytes/snapshot_count/earliest/latest`。

## 4. Storage API（CLI 永不触 SQL）

新增（只读）：
```
get_snapshots_for_root(root, limit) -> Vec<SnapshotRecord>          // DESC
get_history_series(root, path, limit) -> Vec<(id, created_at_ms, size)>
```
`get_history_series` 一条 **LEFT JOIN** 查询返回整条时间序列（§5）。

## 5. history SQL 查询策略（一次查询，无 N+1）

```sql
SELECT s.id, s.created_at, COALESCE(e.size, 0)
FROM snapshots s
LEFT JOIN entries e ON e.snapshot_id = s.id AND e.path = ?
WHERE s.root_path = ?
ORDER BY s.id DESC LIMIT ?
```
- DESC+LIMIT 取"最近 N"，Rust 侧 reverse 成升序 → `--limit` 语义正确。
- LEFT JOIN 保证目录在某次缺失时 size=0 而非丢点。
- `WHERE root_path=?` 严格隔离多 root。
- path 用**存储拼写**（对最新快照做一次 case-insensitive 解析；无则用用户拼写，保底为全 0，文档化为 Windows 边缘）。

## 6. N+1 风险

- history：**单条** LEFT JOIN，无循环查询。
- history <path> 只额外 1 次 case-insensitive 解析（最新快照 1 行），仍 O(1)。
- top/diff/inspect：M2 已有 direct-children 单查询，本阶段不改。

## 7. benchmark generator 设计

- **库**：`src/benchtree/mod.rs` — `generate_tree(root, GenConfig) -> Result<GenStats>`；
  `GenConfig { files: u64, mode: Wide|Deep|Mixed|Tiny|Large, seed: u64, force: bool }`。
  - **确定性**：xorshift64 自研 PRNG（不引 rand），同 seed 同形状（文件名/大小序列一致）。
  - **不写满盘**：Tiny/Mixed 只写 ≤64B 内容；Large 用 `set_len` 稀疏文件；Wide/Deep/Mixed 相对小容量。
  - **精确 N 文件**：生成器计数，循环直到写满 N；目录数按 mode 参数化。
  - 既有非空目录：默认拒绝（防误删用户数据），`--force` 先清理再生成（本机工具，不做自动删除）。
- `examples/generate_tree.rs`：CLI 包装（clap）。
- `examples/bench.rs`：runner —— 生成 → `scanner::scan`（计时/文件每秒）→ `Storage::save_snapshot`（计时 + DB 大小）→ `diff::compute` → `history`，输出表格。
- 10k 为 CI 冒烟；100k/1M 本机跑并记入 `BENCHMARKS.md`（注明 OS/FS/CPU/存储/release/cold-warm 说明；峰值内存"如可靠才报"，本阶段墙钟+速率为主）。

## 8. 当前 schema 是否需要 migration —— **不需要**

snapshots+entries 已含全部所需（时间/root/总量 + 逐目录大小）。retention 设计见 §9，不做物化。

## 9. Retention 设计（只设计，不实现）

- 建议分层（供后续里程碑实现，本阶段绝不自动删除）：
  - 最近 30 天：全量快照。
  - 31–365 天：每日仅保留 1 个。
  - >365 天：每周仅保留 1 个。
- 删除必须显式手动命令 + dry-run；原数据目录仅为 WHY BIG 自管，绝不扫描/删除用户文件。
- 与"no auto data deletion"原则挂钩，进入 M4+ 讨论清单。

## 10. 预计新增依赖 / 测试

依赖：`serde(derive)`、`serde_json`（仅 JSON，不加 rand —— 生成器自研 PRNG）。
测试增量（≈55+）：
- history 15：root / dir / 中途新增 / 中途删除 / 消失又回来 / 多 root / latest-root 选择 / limit / 相对路径 / Unicode / 越界拒绝 / 单快照 / 零快照 / 超大值 / 时间戳排序。
- top 9：最大增长 / shrink 排名 / limit / 全零 / 新增目录 / 删除目录 / 嵌套 attribution / scoped <path> / 多 root。
- JSON(golden) ≈10：valid JSON、schema_version、五命令 schema、stdout 纯 JSON(e2e)、无 ANSI、负 delta、Unicode 路径、时间 RFC3339。
- benchgen ≈5：确定性 / 精确计数 / 非法参数 / 重复生成 / force 清理。
- e2e：snapshot A/B/C → history / history <path> / top / top <path> / diff --json / inspect --json。

## 11. 保持 M2 兼容

diff/inspect 的 domain+human 渲染**不动**；JSON 是并列的新渲染通道。`--json` 是新增可选 flag，human 输出规则与既有 golden 语义完全一致（回归由 M2 测试保证）。

## 12. Phase 4 对抗清单（先写失败测试再修）

history 时间序错 / 缺失目录被丢而非 0 / 多 root 串数据 / top 与 diff attribution 不一致 / 负 delta 排序 / JSON stdout 被污染 / JSON 暴露内部 struct / 溢出 / TZ 歧义 / N+1 / 路径归一化分叉 / Windows 大小写 / 生成器数量不准。
