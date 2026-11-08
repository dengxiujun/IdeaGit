# TEST-003 Verification

## 1. 范围

本记录对应 IP-001 §6.1 的 `TEST-003`，审计 M3 §6.7 要求的真实仓库主链路与失败态。

## 2. 证据矩阵

| 要求 | 证据 | 结果 |
|---|---|---|
| Clone → Edit → Commit → Push → Fetch → Pull | `crates/adapters/git-cli/tests/m3_e2e.rs::m3_gate_clone_edit_commit_push_fetch_pull_merge_conflict_resolve_continue`，使用真实临时仓库、bare remote 与生产 CLI 适配器 | 通过 |
| Merge → Conflict → Resolve → Continue | 同一 E2E；另有 `GitCliConflictStageReader`、`GitCliConflictResolutionWriter` 和 `GitCliSequencerWriter` 实际调用 | 通过 |
| 认证失败 | `credential-broker` AskPass 失败分类测试：进程拒绝映射 `Rejected`，provider 不可用映射 `ProviderFailed`，不泄露 stderr | 通过 |
| 外部竞争 | `m3_gate_rejects_clone_network_failure_and_external_conflict_edit`；工作区摘要变化返回 `StaleSelection`，外部内容保持不变 | 通过 |
| 网络异常 | 同一测试对不可达 HTTPS Clone 验证 `NetworkFailure` | 通过 |
| 崩溃恢复 | `apps/desktop/src/main.rs` 的 `startup_recovery_matches_scope_after_registry_restart` 验证进程重启后重新绑定 session、读取持久 Journal，并在状态不一致时阻断写入；`git-operations` 覆盖未知结果与活动 sequencer 门禁 | 通过 |

## 3. 验证命令

以下命令在本地工作区通过：

```text
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
git diff --check
```

CI 结果以推送 `c59bbb2` 对应的 `Rust CI #104` 为最终跨平台证据。

## 4. 安全边界

- 测试使用隔离临时仓库和本地 bare remote，不访问用户仓库或真实网络远端。
- 未知结果不自动重试；冲突解决在阶段 OID 与工作区摘要变化时拒绝写入。
- 认证测试验证 Credential Broker 的失败分类，不声称已验证真实第三方服务的账号策略。
