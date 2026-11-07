# RECOVERY-000 Verification 001

## 范围

本次实现最小写操作恢复判定，不执行 Git 写操作、不自动重试，也不覆盖真实 Index。

## 验证结果

`classify_recovery` 使用 Journal 记录和一次新的 HEAD/Index 观测，输出四种安全状态：

- `NotExecuted`：HEAD 与 Index 均保持期望值；
- `Committed`：Journal 记录的提交已成为 HEAD，Index 也符合期望；
- `CommittedNeedsReconcile`：提交已成为 HEAD，但 Index 需要后续协调；
- `Unknown`：无法证明属于以上状态，禁止自动继续。

已执行并通过：

```text
cargo fmt --all
cargo test -p git-operations --locked
git diff --check
```

新增测试覆盖未执行、提交后 Index 协调和冲突观测三条路径。

## 风险与后续

本任务只提供纯判定逻辑；真实 Git HEAD/Index reader、Journal 更新、恢复 UI 与可恢复动作由 GIT-001/GIT-002 及后续 RECOVERY 任务接入。未知状态保持人工检查，不做猜测性修复。
