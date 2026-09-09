# WhyBig — Milestone 1 设计文档

> Disk analyzers tell you what's big. WhyBig tells you what **got** big.

本文档是 Milestone 1（`init` / `snapshot` / `status` + Scanner + SQLite Storage + 测试 + CI）的实现前设计，也是评审时对照的规格。

---

## 0. 目标与边界

- 目标：把「目录大小快照」可靠地写进 SQLite，为下一阶段的 `diff` 提供稳定、一致、可比较的数据基础。
- Milestone 1 不实现：`diff` / `inspect` / `history` / `top` / TUI / GUI / daemon / 自动后台扫描 / 删除清理 / AI / 网络 / 云同步 / telemetry / 账号。
- 关键工程指标（工程目标，非产品承诺）：
  - 约 1,000,000 文件、SSD：尽量 ≤ 15s。
  - 内存尽量 < 150 MB。
  - 正确性 > 性能；优化前先 profile（见 `benches/` 规划与 `PERF` 记录）。

---

## 1. 最终模块结构

```
D:\WhyBig\
├── Cargo.toml
├── DESIGN.md            ← 本文档
├── README.md
├── LICENSE              ← MIT
├── .github/workflows/ci.yml
├── src/
│   ├── main.rs          ← 薄入口：解析 → 分派 → 打印，把错误转成用户可读信息
│   ├── cli.rs           ← clap derive：init / snapshot / status + 全局 --data-dir
│   ├── config.rs        ← WhyBig 数据目录解析（env / 平台默认值），无配置文件
│   ├── error.rs         ← 领域错误（thiserror）；CLI 层用 anyhow 组合
│   ├── scanner/         ← 与 SQLite 完全解耦
│   │   ├── mod.rs       ← 公共 API：ScanOptions / scan() / ScanResult
│   │   ├── walker.rs    ← 显式栈式 DFS 遍历（不跟随 symlink、边界检测、排除逻辑）
│   │   ├── entry.rs     ← DirEntry（目录级聚合）
│   │   └── aggregate.rs ← DirAgg（size/file_count/dir_count）+ 汇总统计
│   ├── storage/
│   │   ├── mod.rs       ← Storage 公共 API（init / save_snapshot / list / latest）
│   │   ├── database.rs  ← 连接管理（WAL、FK、busy_timeout、事务助手、路径→字节）
│   │   ├── migrations.rs← PRAGMA user_version 递增迁移
│   │   └── models.rs    ← SnapshotRecord / EntryRecord（i64 内部表示）
│   ├── snapshot/
│   │   ├── mod.rs
│   │   └── service.rs   ← 编排：归一化 root → 扫描(带进度回调) → 事务写入 → 摘要
│   └── output/
│       ├── mod.rs
│       ├── human.rs     ← 面向用户的中文/数字排版（千分位、时长）
│       └── size.rs      ← 人类可读大小（238.7 GB / 730 MB），设定位一致
├── tests/
│   ├── scanner.rs       ← 集成测试：针对文件系统 fixture
│   ├── snapshot.rs      ← 集成测试：init/写入/事务/一致性/FK/损坏
│   └── fixtures/        ← 测试用目录与文件集
└── benches/             ← [后续] 1M 文件基准（先 profiling 再优化）
```

---

## 2. 核心 Rust 结构

```rust
// scanner/aggregate.rs —— 目录级聚合（递归累计）
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirAgg {
    pub total_size: u64,   // 该目录下所有常规文件 apparent size 之和（递归）
    pub file_count: u64,
    pub dir_count: u64,    // 子目录数（递归，不含自身）
    pub skipped: u64,      // 因错误(权限/消失/IO)而未统计的条目数
}

// scanner/entry.rs —— 一个目录的最终记录（持久化单元）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub path: PathBuf,     // 绝对路径（root 已归一化）
    pub agg: DirAgg,
}

// scanner/mod.rs —— 扫描输出
#[derive(Debug, Clone)]
pub struct ScanResult {
    pub root: PathBuf,             // 归一化后绝对路径
    pub entries: Vec<DirEntry>,    // 每个被计入的目录一条（root 在内）
    pub summary: ScanSummary,      // 顶层汇总（与 root.agg 含义相同，供储存储存冗余）
    pub warnings: Vec<ScanWarning> // 每个错误一行（便于以后输出/降级为计数）
}

pub struct ScanSummary {
    pub total_size: u64,
    pub file_count: u64,
    pub dir_count: u64,
    pub skipped_count: u64,  // = ∑ warnings（错误类跳过）
    pub elapsed: Duration,
}

pub enum ScanWarning {
    UnreadableDir { path: PathBuf },
    StatFailed { path: PathBuf, kind: String },
    // 后续可扩展：Vanished... 当前合并为 StatFailed
}

// storage/models.rs —— 持久化记录的内部表示（SQLite INTEGER 是 i64）
pub struct SnapshotRecord {
    pub id: i64,
    pub created_at_ms: i64,
    pub root_path: String,       // lossy UTF-8（见 §12 讨论）
    pub total_size: i64,         // 由 u64 checked 转换
    pub file_count: i64,
    pub dir_count: i64,
    pub scan_duration_ms: i64,
    pub skipped_count: i64,
    pub size_kind: String,       // "apparent"
}

pub struct EntryRecord<'a> {
    pub path: &'a str,
    pub size: i64,
    pub file_count: i64,
    pub dir_count: i64,
}

// storage/mod.rs —— 服务外观
pub struct Storage { conn: Connection }
```

`ScanResult.entries` 只存目录级聚合，**不存任何单文件记录** ⇒ 快照体积≈目录数（一般数万行），内存与 DB 均受控。

---

## 3. Scanner 算法

**遍历方式：显式栈式 DFS（非 OS 递归）**

- 不用递归函数逐步调用（防超深目录导致调用栈溢出），也不用 `ignore` crate：它引入 gitignore 语义且对「不可读目录、EXDEV、自排除」的控制不够透明；我们用 `std::fs::read_dir` + `symlink_metadata` 获得完整、可解释的错误处理。
- 栈帧持有 `std::fs::ReadDir`（owned handle）+ 当前目录路径 + 累计 `DirAgg`；出栈时向上合并并产出该目录的 `DirEntry`。

```
push(root_frame)
loop:
  top = stack.last_mut()
  match top.read_dir.next():
    None => pop; 合并到父帧(dir_count+1, size/文件数累加); 记录 DirEntry
           若为 root 帧 => 结束
    Some(entry) =>
      md = symlink_metadata(entry.path)      // lstat：不解析 symlink
      if Err => top.agg.skipped += 1; 记录 warning; continue
      t = md.file_type()
      if t.is_symlink():
         // 不跟随：不统计、不进目录。作为"链接"从计数里排除（见 §7）
         continue
      if t.is_dir():
         if excluded(当前路径) => 不计入、不下降; 父帧.dir_count+=0（整个子树忽略）
           即：不自增长数据目录 / 越界 mount / 超出 max_depth 的降级处理
         else => 压入新帧（depth+1）
      else: // 常规文件 或 特殊文件(fifo/socket/设备)
         top.agg.file_count += 1
         top.agg.total_size += md.len() as u64
```

关键性质：

- 每文件仅 1 次 `lstat`（size 与类型一次取到）；聚合在内存栈上完成，无 PathBuf 每文件累积。
- 只产出「目录」条目 ⇒ 扫描 1M 文件、6 万目录时 `entries` 仅 ~6 万行。
- 目录被删/不可读：`read_dir` 失败或 `lstat` 失败 → `skipped += 1` + warning，不从整体失败（满足规格 7）。

**边界与排除（walk 期间的 `excluded()`）**

- 排除**不**计为 `skipped`（它们不是错误，是策略），与「权限/消失」类错误分开（见 §13 计数器定义）。
- 排除项：(a) WhyBig 数据目录子树；(b) 与 root 不同 filesystem（Unix 用 `dev()`，见 §6）；(c) `--max-depth` 超限（默认无限制，但保留参数化能力）。

---

## 4. Snapshot 生命周期

```
user 调用 snapshot <root>
─────────────────────────────────────────────────
1. 解析 root：相对路径 → current_dir 连接；绝对化；canonicalize（root 自身 symlink 跟随一次，保证跨快照 key 一致）
2. ensure storage：若数据目录/DB 缺失，自动执行幂等 init（等价 git 自动初始化）
3. 构造 ScanOptions（root、数据目录排除、max_depth=None）
4. 扫描：walk（进度回调 inc）→ ScanResult（含 skipped/warnings）
5. 事务写入：
     BEGIN IMMEDIATE
     INSERT snapshots(...) RETURNING id
     INSERT entries（prepared + finalize/execute，批量）
     COMMIT        （任一步失败 ⇒ ROLLBACK，无半个快照）
6. 打印摘要：文件数 / 目录数 / 总量 / 时长 / warnings 数
```

**进度**：`indicatif::ProgressBar`（stderr），非 TTY 自动隐藏；扫描器通过 `&mut dyn FnMut(u64)` 回调步进计数，与 UI 解耦。

---

## 5. SQLite 事务设计

- 单连接（`Connection`），`PRAGMA journal_mode=WAL`、`foreign_keys=ON`、`busy_timeout=5000`、`synchronous=NORMAL`（WAL 下进程崩溃安全；极端断电风险见 §12 说明）。
- **保存 = 单个事务**：任何一条 `INSERT entries` 失败 ⇒ ROLLBACK ⇒ `snapshots` 中不留该快照（满足「transaction 失败不能留下半个 snapshot」）。
- 迁移也在事务内执行（`PRAGMA user_version` 遍历，每迁移一个事务）。
- 批量插入：prepared statement 复用；6 万行毫秒级，非瓶颈（扫描占绝对大头）。

**Schema（对建议 schema 的两处修改 + 解释）：**

```sql
CREATE TABLE snapshots (
    id INTEGER PRIMARY KEY,
    created_at INTEGER NOT NULL,          -- 纪元毫秒（原设计秒；diff 需要亚秒精度）
    root_path TEXT NOT NULL,
    total_size INTEGER NOT NULL,
    file_count INTEGER NOT NULL,
    dir_count INTEGER NOT NULL,
    scan_duration_ms INTEGER NOT NULL,
    skipped_count INTEGER NOT NULL DEFAULT 0,   -- 新增：status/未来 diff 需要
    size_kind TEXT NOT NULL DEFAULT 'apparent'  -- 新增：为 allocated 留位，值驱动不变式
);

CREATE TABLE entries (
    snapshot_id INTEGER NOT NULL,
    path TEXT NOT NULL,
    size INTEGER NOT NULL,
    file_count INTEGER NOT NULL,
    dir_count INTEGER NOT NULL,
    PRIMARY KEY (snapshot_id, path),
    FOREIGN KEY (snapshot_id) REFERENCES snapshots(id) ON DELETE CASCADE
);

CREATE INDEX idx_entries_path ON entries(path);
```

修改理由：
1. `skipped_count`：规格明确 `status` 要显示「skipped/inaccessible 数量」，而这只可能来自持久化数据。
2. `size_kind`：规格要求不阻碍未来 allocated；用一列显式声明语义，旧数据默认 `apparent`。
3. `created_at` 用毫秒、`scan_duration_ms` 对齐，避免将来 diff 时缺精度。

另：SQLite `INTEGER` 是 i64；`u64` 总量以 checked 转换上限界定（见 §12），8 EiB 内安全。

---

## 6. filesystem / mount boundary 检测

- **Unix（Linux/macOS）**：`std::os::unix::fs::MetadataExt::dev()`——root 的 device id 存起来，遍历中任何 `symlink_metadata().dev() != root_dev` 的目录：不下降、整体忽略（计为排除，非 skipped）。覆盖 EXDEV / 其它挂载点。
- **Windows**：`std` 不暴露 volume serial / reparse 判定 ⇒ Milestone 1 不做跨卷边界检测（junction 可能跨卷；默认行为=跟随下降）。这是记录在案的平台差异（媒体卷/ junction 用户可先预期），列到 §12 与最终报告「未解决跨平台项」。Windows 上仍通过**不跟随 symlink**（std `is_symlink` 对文件 symlink 生效）避免多数循环风险；junction 循环在 NTFS 上概率极低（仅有权限可创建 junction）。

---

## 7. symlink 处理

- 一律 `symlink_metadata`（lstat）→ 永不解析链接目标。
- **链接本身一律不进入任何计数**（file/size/dir 均不含）：symlink→文件 不计 size、不 count；symlink→目录 不下降（目标树完全不统计）；broken symlink **不是错误**、不产生 skipped（lstat 只看链接自身，目标不存在无影响）。
- 因不下降 ⇒ **无法形成 symlink loop 造成的无限递归**（规格 14），无需额外 loop 检测。
- 该策略与 du 的默认有细微差别（du 会统计链接自身的小 size），此处显式偏离并记录：`file_count` 语义保持为「常规/特殊文件数」，更可预期。
- `canonicalize(root)` 仅对**用户显式给出的 root** 生效一次（跟随 root 的 symlink），来自 OS 的便利，与内部遍历策略不冲突。

---

## 8. hard link 处理策略

- 状态：**硬链接按「目录条目」各自计数**（同一 inode 出现 N 次即计入 N 次 size）。
- 理由：(a) 与常见磁盘工具语义一致且可预期；(b) 免去 inode→路径去重表，省内存（1M 文件规模下那张表是百万级条目）；(c) **快照间口径恒定 ⇒ 同一 inode 两侧都计 N 次，diff 的增量正确**（产品核心是增量，不是绝对值）。
- 已文档化：`total_size` 不是「唯一数据占用」而是「目录树 apparent 条目和」，README 会写明。未来可加 `--hardlinks=dedupe`（读 `st_nlink>1` 去重），Milestone 1 不做。

---

## 9. 可能出现的 race conditions（与对策）

| 竞态 | 影响 | 对策 |
|---|---|---|
| 条目在 `read_dir` 与 `lstat` 之间消失 | 仅该文件统计失败 | `lstat` Err → `skipped+1`，不中断 |
| 目录在扫描中删除 | read_dir 失败 | 同上；父目录聚合不受污染 |
| 文件扫描时增长 | 大小是采样时刻值 | 记录为「点时刻采样」，文档说明；快照间天然可比 |
| symlink 目标被替换 | 不解析 ⇒ 无关 | lstat 只看链接本身 |
| root 自身被删除/改名 | open 失败 | 明确报错给用户（唯一允许“失败整体”的位置） |
| 数据目录恰好在 root 下 | 自增长污染 | §13 排除逻辑，逐目录前缀判定 |

---

## 10. 最可能的性能瓶颈（优化前先测量）

1. **每文件 `lstat` 系统调用** —— 1M 文件 ≈ 2M+ syscall（readdir + lstat）。Unix 可用 `d_type`（`DirEntry::file_type` 内部优化）减少 stat；`ignore`/walkdir 也做了同样事。**M1 先正确后快**，`benches/` 用 1M 文件 fixture profiling 后再决定是否上 `libc` d_type 快路径。
2. **PathBuf 分配** —— 栈式遍历只持有当前目录路径 + `read_dir` 自身；不逐个 clone 子路径做 key。`excluded()` 用字符串/组件前缀比对，避免为每目录 canonicalize。
3. SQLite 写入 —— 单事务 + prepared，最少 fsync；不是瓶颈。
4. 进度回调 —— `ProgressBar.inc` 在 1M 次下可接受；若 profile 显示热点，改成每 N 条合并。

rayon：**不引入**。扫描是 I/O 密集、聚合必须确定性；跨线程拆分语义（谁归谁父目录）复杂化且无明确收益，等 benchmark 证明需要再上。

---

## 11. Windows / macOS / Linux 平台差异

| 项 | Linux | macOS | Windows |
|---|---|---|---|
| 路径编码 | UTF-8（可非 UTF-8） | UTF-8（可非 UTF-8） | UTF-16 via OsString |
| `dev()` 跨卷 | ✅ | ✅ | ❌（std 不可得；M1 记录为限制） |
| 大小写敏感性 | 敏感 | 默认不敏感(APFS 可敏感) | 不敏感 |
| 隐藏文件语义 | `.` 前缀 | 同左 | 不同（无统一隐藏概念） |
| 符号链接 | is_symlink ✅ | ✅ | 文件 symlink ✅；junction/联结点 std 不认作 symlink |
| 稀疏文件 | st_size<allocated | 同左 | 同左（allocated 更难取） |
| 根路径分隔 | `/` | `/` | `\` 与 `/` 混合 |

对策：全程 `Path/PathBuf`/`OsString`；排除前缀比较在 Windows 上做大小写不敏感处理（`cfg`）；root 归一化用 `canonicalize`；大小单位格式化平台无关。

**Windows 特判（实测发现）**：本机 `std::fs::canonicalize` 返回 `\\?\C:\...` verbatim 前缀路径。若直接入库/做前缀比较，会与 `%APPDATA%` 拼出来的普通拼写不一致（数据目录排除失效、root key 形态不一）。解法：`snapshot::service` 的 `normalize_abs()` 对 Windows 剥离 `\\?\` / 还原 `\\?\UNC\`，扫描 root、排除路径、入库 key 全部走归一化；Unix 为 no-op。集成测试用 `plain_path` 镜像同一逻辑做断言。

---

## 12. 当前规格存在的问题 / 设计取舍（显式声明）

1. **路径非 UTF-8 → 建议 schema 用 TEXT 有损**。SQLite TEXT 需 UTF-8；Unix 允许非 UTF-8 文件名 ⇒ `to_string_lossy` 会使不同非法字节映射到同一 key（极小概率，但存在）。M1 采用 lossy 并保证**确定性**（同路径同 key，快照间可对齐）；精确字节保持留给未来 BLOB 列迁移。规格第 10 条「不假设路径可安全转 UTF-8」在内部（PathBuf）完全满足，仅持久化键有损。
2. **`created_at` 建议用秒**：diff 需要亚秒；我改毫秒。
3. **schema 未含 skipped_count**：status 规格需要，已加。
4. **SQLite INTEGER=i64 上限**：`u64` 总大小必须 checked 转 `i64`（>8 EiB 用 `i64::MAX` 饱和并记 warning 或报错）。实际无法达到，但代码路径显式处理，防溢出。
5. **性能目标自相矛盾风险**：1M 文件 15s 仅 SSD+快 I/O 可行；严格指标需要先 profile。故「工程目标而非承诺」的表述符合规格（已按此执行）。
6. **「每个目录至少记录」无上界**：60k 目录/快照 × 每行 ~50B 很便宜；但若某用户扫出数百万小目录，M1 仍全部保存（正确性优先）。将来 `diff` 前可加「只保存 ≥ 阈值 或 缩减」策略，M1 不预做。
7. **serde 在 M1 无用**：无配置/序列化需求（状态在 SQLite）。技术栈含 serde，但「不为未来扩展提前引入」，M1 不引；到 diff/config 阶段再加（纯加法，无设计债务）。
8. **ignore crate 不引入**：语义不匹配（无 gitignore 需求）且错误处理不透明；自研 std walker 更可控。规格允许「合适的轮子或自有实现」。
9. **config 文件不写**：所有状态在 SQLite + `--data-dir`/env；M1 无「默认配置」需求，`init` 不生成空配置文件（规格为「必要时」，此处判定为不必要）。

---

## 13. 计数器定义（交付评审用）

- `file_count`：常规文件 + 特殊文件（fifo/socket/设备，size 取 `len()`）；**不含** symlink、不含被排除子树条目。
- `dir_count`：不含 root 自身的所有子目录数（递归）：被下降的目录在其完成时并入父级 (+1)；被策略排除的目录（见下）计为 `+1 dir-only` 后不下钻。**存在被排除目录时 `dir_count` ≠ entries 行数**（那是策略排除，无数据行对其解释合理）。
- `skipped_count`：**错误类**未能统计条目数（lstat/read_dir 失败 = 权限/消失/IO/迭代器错误）；每次都产生一条 `ScanWarning`。
- **symlink（任一目标）**：不计 file/size/dir/skipped——「不下降、不统计」，既非排除也非失败（§7）。
- **策略排除（不计 skipped、无 warning，仅 dir_count+1）**：WhyBig 数据目录子树、跨文件系统目录（Unix `dev()`）、超深目录（`max_depth`，M1 默认不限，参数化保留）。
- 进度回调的 `visited` 含被排除/跳过条目（体现实际遍历量），与计数口径无关。

---

## 14. 测试策略（对应规格清单）

- **单元**：`size.rs`（格式化边界 0/999/1024²…）、`aggregate.rs` 合并、`config.rs` 数据目录解析（env 覆盖）、错误路径排序与用户文案。
- **集成 scanner**（`tests/scanner.rs`，`tempfile` fixture）：
  普通/空/大/稀疏文件、空目录、深层目录(>64)、大量小文件(在单测用 5k 量级)、Unicode/emoji/空格路径、symlink→文件/目录、broken symlink、symlink loop（构造 A→B→A）、permission denied（Windows 上用 ACL 或 `attrib` 跳过？——Windows 权限测试方案见实现注释，Unix 用 `0o000`）、文件/目录扫描中被删（用线程后台删文件 + 大 fixture 制造窗口）、文件增长、hard link（Unix `std::os::unix::fs::link`）、mount boundary（Unix 无法在单测造挂载 ⇒ 用 `dev()` 单测 + 文档说明；Windows 跳过）、相对/绝对/当前目录 root。
- **集成 storage/snapshot**（`tests/snapshot.rs`）：
  `init` 幂等（两次无破坏）、快照写入与回读一致、事务失败不留半个快照（注入 INSERT 失败）、snapshots↔entries 一致性（FK 与行数）、FK 正确（删 snapshots 级联删 entries）、多快照独立、DB 损坏/路径不可写报用户可读错误、超大 size i64 边界测试。

---

## 15. CI（GitHub Actions）

CI test matrix：`ubuntu-latest` 与 `windows-latest`（ubuntu 作为跨平台可移植性/兼容性检查，**不构成官方支持声明**；v0.1.0 官方支持平台为 Windows，发布产物仅 Windows x86_64）。另有 `release-smoke` job 在 ubuntu 上跑发布二进制冒烟。
```
steps: checkout → dtolnay/rust-toolchain@stable (components: clippy, rustfmt)
      → cargo fmt --check
      → cargo clippy --all-targets --all-features -- -D warnings
      → cargo test
```
`--all-features` 现无 feature，保留含义为“全量检查”。Windows job 用 MSVC 默认（与本地一致）。
