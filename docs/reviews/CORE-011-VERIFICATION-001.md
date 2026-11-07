# CORE-011-VERIFICATION-001：Resource Lock Manager 验证记录

- 日期：2026-11-07
- 任务：CORE-011
- 当前状态：本机专项与 Workspace 门禁待本提交完成后确认；本任务不执行真实 Git 写操作。

## 1. 锁模型

`ResourceKey` 由强类型 `ResourceScope` 与 `ResourceKind` 组成：

- `CommonGitDir(RepositoryId) + Refs`：同一 common Git directory 的共享引用资源；
- `Worktree(WorktreeId) + Index`：具体工作区 Index；
- `WorktreeFiles` 与 `Sequencer`：具体工作区文件/时序器资源。

`ResourceLockManager` 使用进程内 Tokio Mutex，并以 scope UUID、scope 类型和资源种类的稳定顺序取得全部锁。`ResourceLock` 持有 `OwnedMutexGuard`，离开作用域自动释放。

## 2. 安全边界

- 多资源请求排序后再获取，避免两个操作以相反顺序等待造成死锁。
- 取消只中断尚未取得的等待，不声称已回滚 Git 副作用。
- 这些锁不是 `.git/index.lock`，不会预占 Git 自己的锁；Stage/Commit 执行时仍须让 Git 自行处理真实 Index 锁。

## 3. 已执行验证

- `cargo fmt --all -- --check`
- `cargo check --workspace --locked`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test --workspace --locked`
- `git-operations` 新增 3 个锁专项测试：同资源串行/独立资源可用、取消等待、跨 scope/kind 全序。

## 4. 遗留风险

- Journal、恢复、真实 Git 写操作和外部进程锁竞争仍由 CORE-012、RECOVERY-000、GIT-001/002 验证。
- 三平台 CI 需在本任务提交推送后确认。
