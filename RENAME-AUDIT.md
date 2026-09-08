# RENAME-AUDIT — WhyBig → DiskDrift

> Scope: first public branding migration + v0.1.0-rc.1 hardening. Feature-frozen.
> Generated before any global search-and-replace. 250 occurrences found across
> the repo (full list: grep spill file, not committed).

## A. 用户可见名称（必须改）
- error messages: "WhyBig is not initialized here. Run `whybig init`" → DiskDrift / diskdrift
- "root lies inside the WhyBig data directory" → DiskDrift data directory
- "no snapshots yet — run `whybig snapshot`" → diskdrift
- migration guard msg "created by a newer version of WhyBig" → DiskDrift
- clap name/about; README; demo prints (`$ whybig …` → `$ diskdrift …`); CHANGELOG(新)

## B. binary / package 名（必须改）
- Cargo.toml `name = "whybig"` → `diskdrift`; description; repository url
- Cargo.lock package entry
- cli.rs `#[command(name="whybig")]` → diskdrift
- examples `name = "whybig-generate-tree" / "whybig-bench"` → diskdrift-*
- tests `env!("CARGO_BIN_EXE_whybig")` → CARGO_BIN_EXE_diskdrift
- `.github/workflows/ci.yml` `./target/release/whybig` → diskdrift; 新增 release workflow
- 冒烟/文档 install 命令 `cargo install diskdrift` / `cargo build --release`, `./target/release/diskdrift`

## C. environment variables
- 正式 `DISKDRIFT_DATA_DIR`；`WHYBIG_DATA_DIR` 保留为 **deprecated legacy 回退**（优先级：--data-dir → DISKDRIFT_DATA_DIR → WHYBIG_DATA_DIR → 默认）。
- 使用 legacy env 时 stderr 输出弃用提示（**JSON stdout 不污染**，需测试）。
- cli.rs 去掉 `#[arg(env=…)]`（env 解析统一放 config），hand-edit。

## D. filesystem paths
- 默认数据目录名 `whybig` → `diskdrift`（各平台对应路径）
- 自排除数据目录名 `.whybig-data` → `.diskdrift-data`（tests/demo 内嵌）
- bench 数据目录 `.whybig-bench-data` → `.diskdrift-bench-data`
- repo 根目录 D:\WhyBig 本身（历史路径，见 J）

## E. database paths
- 默认 DB 文件名 `whybig.sqlite3` → **`diskdrift.db`**（config + tests 断言）
- legacy DB `%APPDATA%\whybig\whybig.sqlite3` → 仅作为 legacy 检测/手工迁移说明

## F. JSON/API 字段（不因 rename 改变）
- `schema_version=1` 保持不变；command 名 snapshot/status/diff/inspect/history/top/prune 不变
- JSON 无 `application/tool=whybig` 字段；不新增
- 唯一影响：status 的 data_dir/database_path 自动显示新路径（无 schema 变化）

## G. tests（must 迁移）
- 所有 `use whybig::…` → `use diskdrift::…`（tests + examples）
- `whybig.sqlite3` 断言 → `diskdrift.db`
- config tests env var → DISKDRIFT_DATA_DIR + 新增 legacy env 回退/弃用测试

## H. docs
- README 整篇重写（DiskDrift 结构）；BENCHMARKS.md 标题
- DESIGN-M1..M4、DESIGN-M2/M3/M4 = **历史文档，保留原 WhyBig 指称**（分类 J）

## I. CI / release
- ci.yml release-smoke binary 名；新增 `.github/workflows/release.yml`（tag v0.1.0-rc.* / v0.1.0，3 平台构建+打包 zip/tar.gz+SHA256SUMS+上传 artifacts）

## J. historical documents / git logs（保留，标注 intentional）
- DESIGN.md / DESIGN-M2.md / DESIGN-M3.md / DESIGN-M4.md 标题与正文中的 WhyBig/whybig 指称
- .git logs（提交作者名/dev email `dev@whybig.local`、提交信息）
- LICENSE 版权行 → 更新为 DiskDrift contributors（非历史）

## K. legacy compatibility concerns（设计决策，见 DESIGN-RC.md）
1. **数据目录**：不自动迁移、不自动 merge。`init` 检测到 legacy `%APPDATA%\whybig` 且新目录不存在 → 友好提示 + 手工迁移说明（复制 `whybig.sqlite3` → `diskdrift.db` 后用新 binary 打开并自动升迁移）。两目录并存 → 提示当前使用磁盘drift 目录，不猜。
2. **legacy env**：DISKDRIFT_DATA_DIR 优先；WHYBIG_DATA_DIR deprecated 回退 + stderr 提示。
3. **DB schema**：不因 rename 新增 migration（meta 表无 app name 字段）；branding ≠ database redesign。

## 执行顺序
1) bulk replace（UTF-8 安全）非历史文件：WhyBig→DiskDrift, whybig→diskdrift, WHYBIG_DATA_DIR→DISKDRIFT_DATA_DIR
2) hand-edit：config.rs（env 优先级+legacy 回退+diskdrift.db+legacy 目录探测）、cli.rs（name/about/去 env attr）、main.rs init legacy 提示
3) tests 收尾（.db 断言、新增 rename/legacy 测试）→ README/CHANGELOG/docs/community/workflow → 验证

## 执行结果（Final）

- Cargo package/lib/bin = `diskdrift` v0.1.0；`cargo metadata` 确认无旧 identity。
- clap name/about、帮助、README、demo、CI（release-smoke 与新增 release.yml）、examples、tests 全部迁移。
- 默认数据目录 `diskdrift`、DB `diskdrift.db`；env `DISKDRIFT_DATA_DIR`，`WHYBIG_DATA_DIR` 仅作 deprecated 回退（stderr 提示，JSON stdout 纯净——已 smoke 验证）。
- legacy 数据目录不自动迁移/不删/不 merge；`init` 检测到则 stderr 提示手工迁移步骤（DESIGN-RC.md）。
- DB schema/JSON schema_version、command 名保持 v1 不变。
- `cargo clean` 后干净重建：仅产出 `diskdrift.exe`。

### 保留的 WhyBig 引用（分类：全部 intentional，无用户可见残留）
1. `src/config.rs` legacy 探测/`WHYBIG_DATA_DIR` 回退 + 弃用文案 + 对应测试。
2. `src/main.rs` init 的 legacy 检测提示文案（stderr）。
3. `src/cli.rs` 顶部关于 legacy env 的文档注释。
4. `DESIGN.md` / `DESIGN-M2.md` / `DESIGN-M3.md` / `DESIGN-M4.md` —— 历史设计文档，保留原代号指称。
5. `RENAME-AUDIT.md` / `DESIGN-RC.md` —— 本次迁移记录本身。
6. `.git` 历史（作者名/ddev email/提交信息）。
7. `tests/rename.rs` 中用于验证 legacy 行为的 WhyBig 字符串（测试数据）。
