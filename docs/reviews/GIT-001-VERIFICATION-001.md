# GIT-001 Verification 001

## 范围

本切片建立 Stage/Unstage 的端口边界、输入校验和 Git CLI typed command。

## 验证结果

- `IndexMutation::Stage` 与 `IndexMutation::Unstage` 使用 `SessionTarget` 和 `RepoPath` 强类型。
- 空路径在 `IndexWriter` 调用前拒绝；含 `[ ]` 等 glob 字符的路径仍作为单个 `RepoPath`，不会被 UI 字符串解释。
- `IndexWriter` 携带 `ExecutionContext`，为后续 OperationCoordinator、锁、取消和 Journal 接入保留边界。
- CLI 适配器只构造 `add -- <paths>` 或 `restore --staged -- <paths>`，不经 Shell；bare 仓库和非 UTF-8 CLI 参数均显式拒绝。

已执行并通过：

```text
cargo fmt --all -- --check
cargo test -p git-ports --locked
cargo test -p git-cli --locked
git diff --check
```

## 后续

真实 Git CLI 的 `add -- <literal paths>`、`restore --staged -- <literal paths>`、Index 锁、外部变化检测和 Snapshot 重建仍需在 GIT-001 后续实现中完成。
