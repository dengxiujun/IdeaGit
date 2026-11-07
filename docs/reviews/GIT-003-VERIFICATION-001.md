# GIT-003 Verification 001

## 范围

本切片实现 BranchWriter 端口和 Git CLI 的分支变更 typed command。

## 验证结果

- 支持 Create、Checkout、Rename、Delete（含 force）四种 `BranchCommand`。
- 分支名来自强类型 `RefName`，作为独立 OS 参数传给 Git，不经 Shell 拼接。
- bare 仓库没有 worktree 时拒绝变更。
- 真实 fixture 覆盖创建 `feature/demo` 后切回 `main`。

已执行并通过：

```text
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
git diff --check
```

后续将接入 Branch 的 Refs 锁、Journal、preflight 和 Snapshot reconcile。

当前已接入 Refs 锁和写后 Snapshot reconcile；Journal 与 preflight 仍待后续切片。

Branch Journal 已接入，分支操作失败或取消不会静默丢失意图；preflight 仍待后续切片。
