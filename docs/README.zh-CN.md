# DiskDrift 使用手册

普通磁盘分析工具告诉你现在什么最大；DiskDrift 记录目录大小随时间的变化，告诉你什么变大了。

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

至少两个快照才能比较变化。先拍一张，隔一段时间再拍一张。

## 安装

从 [DiskDrift Releases](https://github.com/Hakka128/diskdrift/releases) 下载 Windows 压缩包：

```
diskdrift-v0.1.0-x86_64-pc-windows-msvc.zip
```

解压到任意目录，在该目录打开 PowerShell（`Shift` + 右键 → "在终端中打开"）：

```powershell
.\diskdrift.exe --version
.\diskdrift.exe --help
```

命令行程序没有界面，双击只会闪一下窗口就关闭，请在终端里运行。

## 数据目录

数据保存在本地数据库，默认在：

```
%APPDATA%\diskdrift\diskdrift.db
```

需要换位置时，用 `--data-dir` 或环境变量 `DISKDRIFT_DATA_DIR`：

```powershell
.\diskdrift.exe --data-dir D:\disk-data status
$env:DISKDRIFT_DATA_DIR = "D:\disk-data"
.\diskdrift.exe status
```

## snapshot

记录一个目录的大小：

```powershell
.\diskdrift.exe snapshot D:\projects
```

## status

```powershell
.\diskdrift.exe status
```

显示数据目录、数据库大小、快照数量等。

## diff

比较两次快照，把变化归到顶层目录：

```powershell
.\diskdrift.exe diff
.\diskdrift.exe diff --since 7d
.\diskdrift.exe diff --from 1 --to 2
```

默认比较最近两次快照。`--since` 按时间回溯，例如 `7d`（也支持 `30m`、`24h`、`4w`）。`--from/--to` 用快照 ID。同时跟踪多个目录时，diff 只看最近一次快照所在的根目录；那个根目录只有一个快照时会报错，对同一个目录再拍一次就好。

## top

```powershell
.\diskdrift.exe top
.\diskdrift.exe top --since 7d
.\diskdrift.exe top --shrink
```

按变化量列出目录，`--shrink` 反过来按释放的空间排序。

## inspect

```powershell
.\diskdrift.exe inspect D:\projects\app
```

diff 只到顶层；想看某个目录内部时用这个，它列出该目录直接子目录的变化。

## history

```powershell
.\diskdrift.exe history
.\diskdrift.exe history D:\projects\app
```

显示某个目录历次快照的大小趋势。

## prune

```powershell
.\diskdrift.exe prune
.\diskdrift.exe prune --apply
```

清理 DiskDrift 自己的历史快照。默认只预览，加 `--apply` 才实际删除。只删快照记录，不碰你的文件。

## compact

```powershell
.\diskdrift.exe compact
```

删除快照后数据库文件不一定变小，用这个手动压缩。

完整参数看 `.\diskdrift.exe <命令> --help`。

## 扫描行为

- 记录的是目录级别的大小信息，不读取文件内容。
- 符号链接不跟随、不计入。
- Windows 联结（junction）不跟随、不计入。
- 权限不足或文件消失的条目会被跳过，不会中断整个扫描。
- 大小是逻辑大小（apparent byte），与磁盘实际占用可能略有差别。
- 被多次硬链接的文件会在每个目录里各计一次。

## 常见问题

问：双击 exe 一闪就没了？
答：命令行程序，请在 PowerShell 里运行。

问：为什么 diff 说快照不够？
答：要比较的根目录至少要有两个快照。对同一个目录再 snapshot 一次。

问：会删我的文件吗？
答：不会。prune 只删 DiskDrift 自己的快照记录。

问：会联网或后台运行吗？
答：不会。没有网络，没有守护进程，只有你手动运行时才工作。

## 命令速查

```powershell
.\diskdrift.exe snapshot D:\projects
.\diskdrift.exe status
.\diskdrift.exe diff
.\diskdrift.exe diff --since 7d
.\diskdrift.exe top
.\diskdrift.exe top --shrink
.\diskdrift.exe inspect D:\projects\app
.\diskdrift.exe history
.\diskdrift.exe prune
.\diskdrift.exe prune --apply
.\diskdrift.exe compact
```
