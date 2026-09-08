# WhyBig — Milestone 2 设计文档 (diff + inspect)

> 定位不变："Disk analyzers tell you what's big. WhyBig tells you what got big."
> 本阶段只回答：**磁盘空间在两个时点之间发生了什么变化**。不引入任何"智能归因"。

---

## 0. 阶段边界

实现 `whybig diff`（默认 + `--from/--to/--limit/--all`）与 `whybig inspect <path>`。
不实现：history/top/anomaly/TUI/GUI/daemon/自动快照/清理/删除/AI/网络/云/telemetry。
JSON（`--json`）**延期到 M3**：规格优先级 `diff correctness > inspect > tests > JSON`;
serde 仍不引入,避免为可选功能压缩核心正确性的测试预算。

---

## 1. 当前 schema 是否支持 M2 —— 支持,零迁移

- `entries(snapshot_id, path, size, file_count, dir_count)` 存的正是每个**目录**的聚合 size —— diff/inspect 需要的全部数据。
- 沙盘:需求可全部由 `snapshots`(总量/时间/root) + `entries`(逐目录 size) 满足。
- 不做 schema migration(规格:除非实际阻塞;它不阻塞)。
- 需要一个小索引考量:见 §5(查询已按 (snapshot_id, path) PK 前导键走)。

## 2. 当前 path representation 的风险

| 事实 | 处置 |
|---|---|
| 存储为 lossy UTF-8(非 UTF-8 名归一为 U+FFFD),但确定性 | 继续接受;diff 两侧用同一 key 集合,比对仍一致;精确字节迁移留给未来 BLOB |
| 分隔符是**平台原生**:Windows 存 `\`,Unix 存 `/` | 一切层级逻辑用 `std::path::Path::components()`(Windows 同时接受 `\`/`/`),绝不字符串 `starts_with("/a/b")` |
| 前缀碰撞 `/foo/bar` vs `/foo/bar2` | 用**组件级**前缀 + 级数判定“直接子目录”;SQL `LIKE parent||'/%' ESCAPE` 只做窄化,真正判定在 Rust 组件比较 |
| trailing separator / `.` / `..` / 盘符 / UNC / Unicode | components() 天然处理大部分;`..` 的词法解析不做(目标可能已消失,不能 canonicalize);UNC 至少不 crash(components 有 Prefix) |
| Windows 大小写 | 组件比较在 Windows 大小写不敏感(复用 walker 的 comp_eq 逻辑,抽到共享 pathutil) |
| root 归一化 | snapshot root 已是 scanner 归一化形式(含 \\?\ 剥离);inspect 的 target 用同样的 absolutize+剥离(不做文件系统 canonicalize,目录可能已删) |

## 3. u64/i64 策略是否影响 diff —— 不影响(但记录病态限制)

- M1 存库时 `u64_to_i64` 饱和;回读 `i64_to_u64` 精确还原(现实尺寸 < 8 EiB)。
- diff 的 delta 一律用 **`i128`**:`after(u64) - before(u64)` 扩到 i128,永不溢出;
  SnapshotDiff.total_delta / DiffEntry.delta 均为 i128。
- **已知病态限制(显式报告,不静默)**:若某目录尺寸真的 ≥ i64::MAX(8 EiB),两侧都饱和成同一值→delta=0 错误。物理不可达(无文件系统支持),代码不做特殊欺骗,文档记录。
- `size_kind` 两侧都='apparent',不做跨语义比较。

## 4. Attribution / Collapse 实现方案

**纯层级语义,无算法**。核心公式:

- **diff 默认** = 只看 tracked root 的**直接子目录**的 delta(beore/after 合并),
  回答"哪个顶层目录解释了变化"。**不显示 deeper 目录** → 天然无父子双计。
- **inspect <path>** = 目标目录的**直接子目录** delta + **残余 `other`**:
  `other = target_delta - Σ(children_delta)`(i128,可为正/负);
  残余解释"目录自身直接文件 + 未被子目录覆盖的部分"。inspect 启示"下一层的谁"。
- 目录新增:before 缺→按 0;目录删除:after 缺→按 0。
- 排序:grew 按 delta 降序;shrank 按 |delta| 降序(蓝本 section 输出)。
- delta=0 默认不显示;`--all` 显示全部,`--limit N` 截断且提示 "... N more growing/shrinking directories"。
- 确定性:tie-break 一律按 path 词法比较(大小写不敏感平台无关性由比较选择决定)。

## 5. Storage API 设计(CLI 永不触 SQL)

新增读取 API(全部 &self):
```
get_snapshot(id) -> Option<SnapshotRecord>
get_latest_snapshots_for_root(root: &str, limit) -> Vec<SnapshotRecord>   // ORDER BY id DESC
get_entry(snapshot_id, path: &str) -> Option<EntryRecord>                 // PK 点查
get_direct_children(snapshot_id, path: &str) -> Vec<EntryRecord>          // LIKE 窄化 + Rust 组件边界判定
```
- `EntryRecord { path, size: u64, file_count, dir_count }`(读侧模型,size 转 u64)。
- 查询按 `PK(snapshot_id, path)` 前导列,扫描量=目录级,受控(非每文件)。
- LIKE 转义 `\ % _` + `ESCAPE '\'`,Windows 下 `\` 双写;仅作窄化,正确性由 components 过滤兜底。

## 6. Diff Engine 数据结构

```rust
// src/diff/
pub enum DiffState { Added, Removed, Changed }
pub struct DiffEntry {
    pub path: String,        // 全路径 key
    pub before_size: u64,
    pub after_size: u64,
    pub delta: i128,
    pub state: DiffState,
}
pub struct SnapshotDiff {
    pub root: String,            // tracked root(即有界面显示范围)
    pub before: SnapshotRecord,
    pub after: SnapshotRecord,
    pub total_before: u64,
    pub total_after: u64,
    pub total_delta: i128,
    pub grew: Vec<DiffEntry>,    // delta 降序
    pub shrank: Vec<DiffEntry>,  // |delta| 降序
}
compute(storage, before, after, scope_root) -> Result<SnapshotDiff>
```
- selection (默认 diff):`latest_snapshot().root_path` → `get_latest_snapshots_for_root(latest.root, 2)`;
  `<2` → 友好错误；显式 `--from/--to` 校验两快照存在且 **root 相等**(组件级,Windows 大小写不敏感)→ 否则 `Cannot compare snapshots from different roots.`

## 7. Inspect 数据结构

```rust
// src/inspect/
pub struct InspectReport {
    pub target: String,          // 归一化 target 路径
    pub root: String,            // tracked root
    pub before: SnapshotRecord,
    pub after: SnapshotRecord,
    pub target_before: u64,      // 缺省 0
    pub target_after: u64,       // 缺省 0
    pub target_delta: i128,
    pub contributors: Vec<DiffEntry>,  // 直接子目录 delta(降序,含负)
    pub other: i128,             // target_delta - Σ(contributors)
}
inspect(storage, before, after, root, target) -> Result<InspectReport>
```
- target 解析:相对→cwd 连接;剥离 \\?\;**必须 is_under(target, root)** 否则报"outside tracked root"。
- both 不存在 → "directory <path> is not present in either snapshot"(引导用户查 diff)。
- new dir(before 缺) / deleted dir(after 缺) 分别按 0 处理,contributors 只有一侧集合。

## 8. 计划新增测试(≈40)

- **pathutil 单元**:直接子级判定、前缀碰撞 foo/bar vs foo/bar2、trailing sep、盘符/UNC 不 crash、Windows 大小写、Unicode。
- **diff 集成(15)**:identical、单目录增/缩、目录新增/删除、多目录、排序、limit、同 root 选择、多 root、跨 root 显式拒绝、<2 快照、超大尺寸(i128)、零 delta、root 自身增长。
- **attribution(10)**:父子同增、仅孙级增、兄弟目录、直接文件→parent other 正/负、parent 被删、child 新增、深层嵌套、前缀碰撞。
- **inspect 集成(11)**:常规/root 自身/新目录/已删目录/两快照皆无/越界/相对路径/Unicode/Windows 层级/仅直接子级/other 计算。
- **e2e(真实 temp fs)**:snapshot A→改动→snapshot B→`diff` 输出断言;`inspect` 下钻。

## 9. 模块组织

```
src/
├── pathutil.rs                 # 共享组件级路径工具(从 walker 抽出 is_under/comp_eq + is_direct_child)
├── diff/
│   ├── mod.rs                  # SnapshotDiff/DiffEntry/DiffState + compute + selection
│   └── select.rs               # 默认/显式选快照(含跨 root 校验)
├── inspect/
│   └── mod.rs                  # InspectReport + inspect()
├── storage/ (扩)               # EntryRecord + 4 个读 API
└── output/
    ├── diff.rs                 # diff 渲染(Total/Grew/Shrank + limit+"N more")
    └── inspect.rs              # inspect 渲染(header/timestamps/contributors/other)
```
CLI:`whybig diff [--from ID --to ID] [--limit N] [--all]`、`whybig inspect <PATH>`。
walker 抽出 pathutil 为**搬家式重构**(行为不变),其余 M1 不动。

## 10. 审查重点(Phase 4 对表)

double counting / attribution / 前缀 bug / added-removed / overflow(i128) /
cross-root / snapshot selection / 负 delta / other 计算 / 平台路径 / SQL 走索引。
