# CORE-012 Verification 001

## 范围

本次实现 Operation Journal 的领域记录、端口契约和 JSON 持久化适配器，不改变 UI 或 Git 写操作流程。

## 验证结果

- `JournalRecord` 使用 schema version 1，覆盖操作身份、目标、命令、期望状态、Index 备份、候选 Tree、实际提交及阶段/恢复结果。
- `JournalStore` 提供 append、replace、get 和按 `SessionTarget` 查询待恢复记录；重复 ID 与不存在记录的替换均显式失败。
- `JsonJournalStore` 对单实例写入串行化，写入临时文件后原子替换目标文件；缺失文件按空日志处理，未知 schema 被拒绝。
- 测试覆盖存储重启后的记录恢复、目标过滤、完成记录排除、重复追加和缺失替换。

已执行并通过：

```text
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
git diff --check
```

## 风险与后续

当前写锁是进程内的；跨进程并发写入协调以及崩溃恢复执行器由 `RECOVERY-000` 和后续写操作任务负责。Windows 上的替换语义仍需在三平台 Runner 中复验。
