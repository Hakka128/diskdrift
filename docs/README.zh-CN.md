# DiskDrift 中文使用手册

普通磁盘分析工具告诉你什么**最大**；DiskDrift 告诉你什么**变大了**。

> 当前文档版本：v0.1.0（Windows x86_64）

[English README](../README.md)

---

## 1. 简介

DiskDrift 是一个本地优先（local-first）的**磁盘增长调试器**。它会定期记录某个目录的"快照"，然后在两次快照之间告诉你：**哪些目录变大了、变大了多少、是最近什么时候变大的**。

- 不是 `du` 的替代品：`du` 只看"现在谁最大"；DiskDrift 看重"谁在变大"。
- 只保存**目录级别**的大小数据（一个快照通常只有几十 KB），不保存文件内容。
- 完全离线运行：不联网、不上传、不收集遥测数据。

官方支持平台：**Windows x86_64**（v0.1.0）。

## 2. 快速开始

假设你担心某个目录（例如 `D:\projects` 或你的用户目录）在缓慢变大：

```powershell
# 1) 初始化数据目录（第一次使用）
.\diskdrift.exe init

# 2) 拍第一张快照（此刻的"基线"）
.\diskdrift.exe snapshot D:\projects

# ……几天或几周后，目录里新增了东西……

# 3) 再拍一张
.\diskdrift.exe snapshot D:\projects

# 4) 查看这段时间发生了什么
.\diskdrift.exe diff
.\diskdrift.exe top
```

关键是：DiskDrift 需要"**至少两张相隔一段时间的历史快照**"才能回答"什么变大了"。只拍一次是无法得到变化的。

## 3. 安装

### 从 GitHub Releases 下载（推荐）

1. 打开 [DiskDrift Releases](https://github.com/Hakka128/diskdrift/releases) 页面，下载 Windows 压缩包：

   ```
   diskdrift-v0.1.0-x86_64-pc-windows-msvc.zip
   ```

2. 解压到一个目录（例如 `D:\tools\diskdrift`）。
3. 在该目录打开 **PowerShell**（在资源管理器空白处 `Shift` + 右键 → "在终端中打开"，或在开始菜单搜索 PowerShell 后 `cd` 过去）。
4. 验证：

   ```powershell
   .\diskdrift.exe --version
   .\diskdrift.exe --help
   ```

> **双击没反应？** `diskdrift.exe` 是命令行程序，不是图形界面。双击它通常只会"闪一下窗口就关闭"——因为它输出完帮助后立即退出。请在**终端（PowerShell / Windows Terminal）里运行**。

### 从源码安装

```powershell
cargo build --release
.\target\release\diskdrift.exe --help
```

> 说明：**`cargo install diskdrift` 目前还不可用**。该命令会在 crates.io 正式发布后开放；发布前请使用上面的下载或源码构建方式。

## 4. 工作原理

```
文件系统
  │  扫描（只统计目录与文件大小）
  ▼
快照 #1（SQLite 数据库，仅目录级别）
  │  时间推移，磁盘发生变化
  ▼
快照 #2
  │
  ├── diff / top     → 两级之间，哪些顶层目录变了 / 谁涨最多
  ├── inspect <目录> → 逐层下钻，看某个目录内部谁在涨
  └── history        → 一个目录随时间的变化趋势
```

- 数据保存在本地 SQLite 数据库 `diskdrift.db` 中（详见第 13 节）。
- "变大了多少"由相邻两次快照的差值计算得出，差值以字节为单位精确统计。

## 5. snapshot — 创建磁盘快照

记录一个目录树此刻的总大小与各子目录大小。

```powershell
.\diskdrift.exe snapshot D:\projects
```

可选参数：

- `--json`：以 JSON 输出（适合脚本）；使用 JSON 时不显示进度条。
- `--data-dir <DIR>`：指定数据目录（见第 13 节）。

> 路径可以是绝对路径，也可以是相对路径。DiskDrift 会记住它扫描的根目录。

## 6. status — 查看状态

查看数据目录位置、数据库大小、快照数量、最早/最新快照。

```powershell
.\diskdrift.exe status
```

可选参数：`--json`。

## 7. diff — 比较磁盘变化

比较两个快照，把增长/缩减**归因到顶层目录**。

```powershell
# 最近两个快照（默认）
.\diskdrift.exe diff

# 只看最近约 7 天
.\diskdrift.exe diff --since 7d

# 指定两个快照（按快照 ID；ID 可在 --json 输出的 snapshot_id 中看到）
.\diskdrift.exe diff --from 1 --to 2
```

可用参数：

| 参数 | 含义 |
|---|---|
| `--from <ID>` | 较旧快照 ID（需与 `--to` 一起用） |
| `--to <ID>` | 较新快照 ID（需与 `--from` 一起用） |
| `--since <DUR>` | 按时间回溯比较，如 `30m`、`24h`、`7d`、`4w` |
| `--limit <N>` | 每组最多显示多少条（默认 10） |
| `--all` | 显示所有变化条目（覆盖 `--limit`） |
| `--json` | JSON 输出 |

> 多根目录时：不带参数时比较的是**最近一次快照所属根目录**的两个最新快照；`--from/--to` 必须属于同一个根目录，跨不同根目录会报错。

## 8. top — 找出增长 / 缩减最大的目录

按变化量排序，快速回答"最近谁涨得最多"。

```powershell
# 最近两个快照中增长最大的目录
.\diskdrift.exe top

# 最近 7 天
.\diskdrift.exe top --since 7d

# 反转视角：谁释放了最多空间
.\diskdrift.exe top --shrink
```

可用参数：`--shrink`、`--limit <N>`（默认 10）、`--since <DUR>`、`--json`。

## 9. inspect — 深入分析目录增长

`diff` 只看顶层；想看清"这个块内部到底是谁在涨"时用下钻分析：

```powershell
.\diskdrift.exe inspect D:\projects\app
```

它会列出该目录的**直接子目录**各自的变化量，以及一个 `other` 余量（直接文件或统计差额）。

可用参数：`--limit <N>`（默认全部）、`--json`。

## 10. history — 查看目录历史

查看某个目录在历次快照中的大小趋势（无参数时默认看被跟踪的根目录本身）。

```powershell
.\diskdrift.exe history
.\diskdrift.exe history D:\projects\app
```

可用参数：`--limit <N>`（默认 20 个快照）、`--json`。

## 11. prune — 管理历史快照

快照积累多了，可以用保留策略清理 **DiskDrift 自己的历史快照**（见安全性说明，它绝不动你的文件）。

```powershell
# 先预览会保留 / 删除什么（默认是"干跑"，不会删任何东西）
.\diskdrift.exe prune

# 确认无误后真正执行
.\diskdrift.exe prune --apply
```

策略参数（默认值）：

| 参数 | 默认 | 含义 |
|---|---|---|
| `--recent-days <N>` | 7 | 最近 N 天内的快照全部保留 |
| `--daily-days <N>` | 30 | 30 天内的快照每天各保留一个 |
| `--weekly-days <N>` | 365 | 365 天内每周各保留一个 |

任何根目录**最新的一个快照**始终保留，不会被删除。

## 12. compact — 压缩数据库

删除快照后，SQLite 文件不一定立刻变小；`compact` 会显式执行数据库压缩（VACUUM）：

```powershell
.\diskdrift.exe compact
```

> 压缩需要约等于数据库大小的临时磁盘空间；在执行前请确保磁盘有余量。

## 13. 数据目录与 DISKDRIFT_DATA_DIR

DiskDrift 的数据存在**数据目录**里，包含数据库文件 `diskdrift.db`。

默认位置（Windows）：`%APPDATA%\diskdrift\`

查找顺序（优先级从高到低）：

1. `--data-dir <目录>`（命令行参数）
2. 环境变量 `DISKDRIFT_DATA_DIR`
3. ~~`WHYBIG_DATA_DIR`~~（**已废弃**的旧版兼容环境变量，仅作为过渡；设置它会打印一条弃用提示，请改用 `DISKDRIFT_DATA_DIR`）
4. 平台默认位置

示例：

```powershell
.\diskdrift.exe --data-dir D:\disk-data status
$env:DISKDRIFT_DATA_DIR = "D:\disk-data"
.\diskdrift.exe status
```

> 用 `status` 查看当前实际使用的数据目录与数据库路径。

## 14. JSON 输出

以下命令支持 `--json`：`snapshot`、`status`、`diff`、`inspect`、`history`、`top`、`prune`。

- 输出是**单个 JSON 文档**，所有提示、进度条、警告都只走 stderr，不会污染 stdout。
- `schema_version` 当前为 `1`。
- 大小为整数（字节）；时间戳为 RFC3339 UTC 格式。

```powershell
.\diskdrift.exe top --json | ConvertFrom-Json
```

JSON 适合脚本与自动化处理：输出为结构化、机器可读的数据。交互式查看时，普通人可读的文本输出通常更方便。

> 注意：目前项目不保证在 v0.1.0 之后 JSON schema 永不变化；脚本应依据 `schema_version` 字段做兼容判断。

## 15. 扫描行为与限制

为了让文档可信，这里如实说明 v0.1.0 的扫描行为与已知限制：

- **符号链接（symlink）不会被跟随，也不会被计入**。
- **Windows 联结（junction）**：与符号链接一样，不会被跟随、也不会被计入；指向其他卷的联结同样会被跳过，不会穿越到目标卷。
- **权限不足 / 文件消失**：这类条目会被记入 "skipped"，不会导致扫描失败——快照是"尽力而为的采样"，不是原子操作。
- **大小为逻辑（apparent）字节数**：与磁盘占用（allocated）不同。
- **硬链接**按其在每个目录中的出现分别计数。
- **只做目录级聚合**：DiskDrift 告诉你是"哪个目录"变大了，而不是"具体哪一个文件"（这是刻意的设计取舍）。
- **非 UTF-8 文件名**会以确定的 lossy UTF-8 形式存储；**Windows 大小写折叠**存在一些边角情况（例如大小写不同且已不存在的路径在 history 中可能读到 0）。
- `--since` 只接受整数单位（`m` / `h` / `d` / `w`），不支持"几个月"这样的日历计算。

## 16. 常见问题

**Q：双击 diskdrift.exe 只闪了一下就没了？**
A：它是命令行程序。请在 PowerShell / Windows Terminal 里运行；双击时窗口在输出完成后立即关闭是正常现象。

**Q：为什么 `diff` 提示"快照不够"？**
A：`diff`、`top` 默认比较**最近一次快照所属根目录**的最新两个快照；如果该根目录只有 1 个快照，会提示 `need at least two snapshots of root ... (only N available)`。请对**同一个目录**再执行一次 `snapshot`。若要比较其他根目录，需要先对该目录多拍几次快照，或使用属于**同一根目录**的 `--from/--to`；跨不同根目录的显式比较会报 `Cannot compare snapshots from different roots`。

**Q：会删除我的文件吗？**
A：不会。DiskDrift 是**观察/调试**工具。`prune` 只管理 DiskDrift 自己的历史快照记录，绝不会删除或修改你扫描过的真实文件。

**Q：数据会发到网上去吗？**
A：不会。本地优先，无网络、无云、无遥测。

**Q：它是后台服务吗？会自己偷偷跑吗？**
A：不是。DiskDrift 只有你手动运行它时才工作，没有守护进程、没有文件监视器。

## 17. 常用命令速查

```powershell
.\diskdrift.exe init                                  # 初始化
.\diskdrift.exe snapshot D:\projects                  # 记录基线
.\diskdrift.exe snapshot D:\projects                  # 过段时间再记录
.\diskdrift.exe diff                                  # 最近两个快照谁变了
.\diskdrift.exe diff --since 7d                       # 近 7 天
.\diskdrift.exe top                                   # 谁涨最多
.\diskdrift.exe top --shrink                          # 谁释放最多
.\diskdrift.exe top --since 7d                        # 近 7 天版本
.\diskdrift.exe inspect D:\projects\app               # 下钻某目录
.\diskdrift.exe history                               # 根目录趋势
.\diskdrift.exe history D:\projects\app               # 某目录趋势
.\diskdrift.exe prune                                 # 保留策略预览（干跑）
.\diskdrift.exe prune --apply                         # 执行清理（仅自身快照）
.\diskdrift.exe compact                               # 压缩数据库
.\diskdrift.exe status                                # 查看状态
.\diskdrift.exe top --json | ConvertFrom-Json         # 脚本化输出
```

---

DiskDrift 负责记录、比较、解释磁盘增长。它不会帮你清理电脑、不会读取文件内容、不会把你的数据发到任何地方。
