# ADR-RECOVERY-IDENTITY-001：跨进程 Journal 仓库身份

- 状态：Draft（待项目负责人批准）
- 日期：2026-11-07
- 关联任务：M2 `RECOVERY-000`
- 影响决策：`ADR-ARCH-REMEDIATION-001` D2、D4，Git Core §18.5

## 1. 问题背景

`RECOVERY-000` 要求应用重启后根据未完成 Journal 和真实 HEAD/Index 判断写操作是否执行。当前 `JournalRecord` 仅以 `SessionTarget` 标识目标；其中 `RepositoryId`、`WorktreeId` 和 session generation 都由进程内 `RepositoryRegistry` 创建。

仓库重启后会得到新的随机 ID。`JournalStore::pending(current_target)` 因而无法返回上次进程留下的记录，即使两次打开的是同一个 Git `common_dir` 和 worktree。现有纯分类器与真实状态 reader 均无法弥补这项身份缺失。

## 2. 阻断原因

- 若继续按 `SessionTarget` 查询，启动恢复在真实崩溃重启场景恒定漏检，不能开放 Stage、Commit 或 Branch 写入口。
- 让现有 `RepositoryId/WorktreeId` 直接由路径生成会改变已批准的会话身份语义，并把会话世代与持久身份混为一体。
- 给 `JournalRecord` 增加持久仓库身份会修改核心数据模型和 Journal schema。按根 `AGENTS.md`，获得批准前不得实施。
- Git 路径可能非 UTF-8；不能用展示字符串、lossy 字符串或当前目录推导身份。

## 3. 候选方案

### A：保持当前 `SessionTarget`（不推荐）

不修改模型，仅检查当前进程产生的 Journal。改动最小，但无法满足“启动恢复”，崩溃后会静默漏掉未知写结果。

### B：把会话 ID 改成规范化路径哈希（不推荐）

由 `common_dir` 和 worktree 路径生成 `RepositoryId/WorktreeId`，使现有字段跨进程稳定。它侵入所有会话、UI key、资源锁和缓存语义；路径移动、大小写与符号链接差异也会改变 ID，且无法保留“每次打开的新世代”语义。

### C：Journal 增加独立持久恢复作用域（推荐）

保持 `SessionTarget` 为进程内、世代限定的执行身份；新增与其正交的 `RecoveryScope`，由 Git discovery 返回的规范化 `common_dir` 与具体 `git_dir`/worktree 身份生成。Journal 写前同时记录执行 target 和恢复 scope；启动时按当前 discovery 的 scope 查询旧记录，再把检查结果绑定到新的 session target。

路径身份采用平台无损编码或等价的版本化强指纹，不经过 UTF-8 展示字符串。原始路径只用于本机匹配和诊断，不发送到遥测。找不到匹配仓库的旧记录保留为 orphaned Journal，不自动删除或重试。

## 4. 方案对比

| 维度 | A：现状 | B：稳定会话 ID | C：独立恢复作用域 |
|---|---|---|---|
| 真正跨进程恢复 | 否 | 是 | 是 |
| 保持会话世代语义 | 是 | 否 | 是 |
| 对资源锁/UI 身份影响 | 无 | 高 | 低 |
| 仓库移动后的处理 | 漏检 | 变成新 ID | 明确 orphaned、人工检查 |
| 非 UTF-8 路径 | 无法解决 | 需另行处理 | 契约明确要求无损编码 |
| Journal schema 迁移 | 无 | 可能无需 | 需要 v1→v2 安全迁移 |

## 5. 推荐方案

批准方案 C，并约束如下：

1. 在 `git-domain` 增加不可与 `SessionTarget` 混用的 `RecoveryScope` 强类型；其输入只来自已规范化的 `RepositoryLocation`。
2. `RecoveryScope` 同时区分共享 `common_dir` 和具体 worktree/`git_dir`，避免 linked worktree 的 Index 记录串仓；bare 仓库有独立形态。
3. `JournalRecord` schema 升级到 v2 并持久化 recovery scope。v1 未完成记录不能假定属于当前仓库，统一显示为 orphaned/需要人工检查；不得丢弃或自动迁移成当前 scope。
4. `JournalStore` 增加按 recovery scope 查询 pending 记录的接口；恢复检查输出旧 operation、旧 target、当前 target 和 disposition，但不得用旧 target 执行新命令。
5. Stage/Commit/Branch 写前必须已取得 scope 并成功耐久化；scope 缺失、Journal 解码失败、存在匹配的 `Unknown` 或 `CommittedNeedsReconcile` 时，当前仓库写入口保持关闭。
6. 恢复仅核对真实 HEAD/Index；不自动重试 Commit、不覆盖 Index、不静默删除 orphaned/未知记录。

## 6. 影响范围

- `git-domain`：新增 `RecoveryScope`；`JournalRecord` 与 schema v2。
- `git-ports`：Journal pending 查询从 session target 改为 recovery scope。
- `git-application` 或 desktop composition root：由 `RepositoryLocation` 构造 scope，并把恢复结果重新绑定当前 session。
- `persistence`：v1/v2 解码、orphaned 记录保留、按 scope 查询。
- `git-operations`：启动检查器输入 current target + scope；分类器仍保持只读。
- `LLD-GIT-CORE-001.md`、`IP-001-development-plan.md`、`PROGRESS.md`：批准后补充持久恢复身份、迁移和开放门禁。

不更换 Rust、GPUI、Git CLI 或持久化技术栈，不改变模块依赖方向。

## 7. 验证标准

- 进程 A 在真实 fixture 写入 pending Journal，销毁全部 registry/runtime；进程 B 重新 discovery/open 后能定位同一记录。
- 同一 `common_dir` 的两个 linked worktree 不互相读取 Index/Commit pending 记录。
- 普通仓库、linked worktree、bare、符号链接规范化以及 Unix 非 UTF-8/Windows 宽路径编码有测试；不使用 lossy 展示路径参与匹配。
- v1 完成记录可继续读取；v1 未完成记录保留且进入 orphaned/人工检查，不被当作当前仓库记录。
- Journal 截断、未知 schema、匹配记录为 `Unknown` 时写入口保持关闭，且错误可诊断。
- `cargo fmt --all -- --check`、workspace locked check、Clippy、tests 和三平台 Rust CI 通过。
- 真实 Git 测试证明恢复过程不创建 Git 写进程、不重试 Commit、不修改 HEAD/Index/refs。

## 8. 批准后执行顺序

1. 更新 Git Core LLD 和 IP/PROGRESS 中的身份与迁移约束。
2. 实现 `RecoveryScope`、Journal v2 与安全读取迁移。
3. 接入真实 desktop 启动检查和写 capability gate。
4. 增加跨 registry 重启、linked worktree、损坏 Journal 与未知结果测试。
5. 通过本机门禁后推送，执行三平台 Rust CI。

批准本 ADR 仅授权上述身份模型与迁移，不代表 `RECOVERY-000` 或 M2 写能力已验收完成。
