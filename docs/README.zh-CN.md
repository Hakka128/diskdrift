# DiskDrift 使用手册

普通磁盘分析工具告诉你现在什么最大；DiskDrift 记录一段时间里目录大小的变化，告诉你什么变大了。

> v0.1.0 · Windows x86_64
> [English README](../README.md)

## 快速开始

```powershell
.\diskdrift.exe snapshot D:\projects

# 过一段时间，目录里有了变化
.\diskdrift.exe snapshot D:\projects

.\diskdrift.exe diff
.\diskdrift.exe top
```

`snapshot` 把目录大小记进本地数据库，首次运行会自动创建数据目录和数据库。至少两个快照才能比较变化，所以先拍一张，等目录变了再拍一张。`init` 只是打印数据目录位置，不是必需的。

## 安装

从 [DiskDrift Releases](https://github.com/Hakka128/diskdrift/releases) 下载 Windows 压缩包：

```
diskdrift-v0.1.0-x86_64-pc-windows-msvc.zip
```

解压到任意目录，在资源管理器里 `Shift` + 右键 → "在终端中打开"（PowerShell），然后：

```powershell
.\diskdrift.exe --version
.\diskdrift.exe --help
```

命令行程序没有界面。双击 exe 只会看到窗口闪一下然后关闭，请放在终端里运行。

源码构建：

```powershell
cargo build --release
.\target\release\diskdrift.exe --help
```

`cargo install diskdrift` 要等 crates.io 正式发布之后才可用。

## 数据目录

数据存在本地 SQLite 数据库里。Windows 默认位置是 `%APPDATA%\diskdrift\diskdrift.db`。

数据目录查找顺序：

1. `--data-dir <目录>`
2. 环境变量 `DISKDRIFT_DATA_DIR`
3. `WHYBIG_DATA_DIR`（旧版兼容变量，已废弃；使用时会打印弃用提示）
4. 默认位置

```powershell
.\diskdrift.exe status
$env:DISKDRIFT_DATA_DIR = "D:\disk-data"
.\diskdrift.exe status
```

`status` 会显示当前实际用的数据目录和数据库路径。

## snapshot

记录一个目录的大小：

```powershell
.\diskdrift.exe snapshot D:\projects
```

快照只保存目录级别的大小数据，不读取文件内容，通常一个快照几十到几百 KB。

## status

```powershell
.\diskdrift.exe status
```

显示数据目录、数据库大小、快照数量和最早/最新快照。

## diff

比较两次快照，把变化归到顶层目录：

```powershell
.\diskdrift.exe diff
.\diskdrift.exe diff --since 7d
.\diskdrift.exe diff --from 1 --to 2
```

- 默认比较最近两次快照。
- `--since` 接受 `30m`、`24h`、`7d`、`4w`。
- `--from/--to` 用快照 ID，ID 可以在 `--json` 输出的 `snapshot_id` 里看到。两个 ID 必须属于同一个根目录，跨根会报 `Cannot compare snapshots from different roots`。
- 同时跟踪多个根目录时，`diff` 只看最近一次快照所在的根目录；如果那个根目录只有一个快照，会提示 `need at least two snapshots of root ...`，对它再拍一次就好。

## inspect

`diff` 只到顶层。想看某个目录内部：

```powershell
.\diskdrift.exe inspect D:\projects\app
```

列出该目录直接子目录各自的变化，还有一个 `other` 余量用来覆盖直接放在目录里的文件。

## history

```powershell
.\diskdrift.exe history
.\diskdrift.exe history D:\projects\app
```

显示某个目录历次快照的大小趋势，默认最近 20 条。

## top

按变化量列出目录：

```powershell
.\diskdrift.exe top
.\diskdrift.exe top --since 7d
.\diskdrift.exe top --shrink
```

`--shrink` 反过来按释放的空间排序。

## prune

清理 DiskDrift 自己的历史快照。默认只预览，不会删任何东西：

```powershell
.\diskdrift.exe prune
.\diskdrift.exe prune --apply
```

按时间分层保留：最近 7 天全部保留（`--recent-days`），30 天内每天保留一条（`--daily-days`），365 天内每周保留一条（`--weekly-days`）。每个根目录最新的快照始终保留。

`prune` 只删快照记录，不碰你扫描过的文件。

## compact

删掉快照之后数据库文件不一定变小，可以手动压缩：

```powershell
.\diskdrift.exe compact
```

内部是 SQLite VACUUM，需要大约数据库大小的临时空间，执行前确认磁盘有余量。

## JSON

`snapshot`、`status`、`diff`、`inspect`、`history`、`top`、`prune` 都支持 `--json`：

```powershell
.\diskdrift.exe top --json | ConvertFrom-Json
```

- `schema_version` 现在是 `1`。
- JSON 和人类可读输出是同一批数据，适合脚本处理。
- 提示和警告只走 stderr，不会混进 JSON，stdout 始终是干净的一个文档。
- 大小用整数字节，时间戳是 RFC3339 UTC。

先别把 schema 当成永远不变的接口来写死，脚本里判断一下 `schema_version` 更稳。

## 扫描行为

- 符号链接不跟随、不计入。
- Windows 联结（junction）同样跳过，不会穿到目标卷。
- 权限不足或文件中途消失的条目记入 skipped，不会中断扫描。
- 大小是逻辑字节数（apparent），不是磁盘占用。
- 硬链接在它出现的每个目录里各算一次。
- 只存目录级别的聚合，不追到具体哪个文件。
- 非 UTF-8 文件名按 lossy UTF-8 存储；Windows 大小写折叠在个别情况下可能把已不存在的路径读到 0。
- `--since` 只接受整数单位，没有"几个月"之类的日历计算。

## 常见问题

问：双击 exe 一闪就没了？
答：命令行程序，需要在 PowerShell 里运行。

问：为什么 `diff` 说快照不够？
答：要比较的根目录至少要有两个快照。对同一个目录再 `snapshot` 一次。

问：会删我的文件吗？
答：不会。`prune` 只删 DiskDrift 自己的快照记录。

问：会联网或者后台运行吗？
答：不会。没有网络，没有守护进程，只有你手动运行时才工作。

## 命令速查

```powershell
.\diskdrift.exe init                                  # 初始化
.\diskdrift.exe snapshot D:\projects                  # 记录快照
.\diskdrift.exe status                                # 查看状态
.\diskdrift.exe diff                                  # 最近两次快照的变化
.\diskdrift.exe diff --since 7d                       # 近 7 天
.\diskdrift.exe inspect D:\projects\app               # 下钻某个目录
.\diskdrift.exe history                               # 目录历史趋势
.\diskdrift.exe top                                   # 增长最大的目录
.\diskdrift.exe top --shrink                          # 释放空间最多的目录
.\diskdrift.exe prune                                 # 预览保留策略
.\diskdrift.exe prune --apply                         # 实际执行清理（仅自身快照）
.\diskdrift.exe compact                               # 压缩数据库
.\diskdrift.exe top --json | ConvertFrom-Json         # 脚本用 JSON 输出
```
