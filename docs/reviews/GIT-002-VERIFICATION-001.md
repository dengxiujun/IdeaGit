# GIT-002 Verification 001

## 范围

本切片实现 Standard Commit 的端口和 Git CLI 适配器。

## 验证结果

- `CommitWriter` 返回实际 commit OID 和父提交列表。
- commit message 通过 stdin 传给 `git commit --file=-`，不拼接 shell 命令；支持 amend、allow-empty、signoff 与签名策略。
- 提交后使用 `rev-parse HEAD` 和 `rev-list -1 --parents HEAD` 获取实际结果；Git 拒绝映射为结构化错误。
- 真实 fixture 覆盖 staged 文件修改提交和单父提交结果。
- `PreflightCommitWriter` 在 CLI 启动前比较 expected HEAD/Index，变化分别返回 `HeadChangedExternally` 或 `IndexChangedExternally`。
- `CoordinatedCommitWriter` 保护共享 Refs 与具体 Worktree Index，使用既有稳定排序和取消感知锁管理器。
- `JournaledCommitWriter` 持久化 CommitInProgress、实际 OID/parents 和 Completed/RecoveryRequired 终态，避免 Hook/进程异常后丢失结果。
- `ReconciledCommitWriter` 将实际 CommitOutcome 与写后 Snapshot 绑定；Snapshot 重建失败不会发布完整成功。

已执行并通过：

```text
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
git diff --check
```

## 后续

Commit 的 expected HEAD/Index preflight、Journal/锁/Reconcile 组合和 Hook 拒绝故障矩阵将在后续 GIT-002 切片完成。
