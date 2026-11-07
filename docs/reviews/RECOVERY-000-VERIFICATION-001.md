# RECOVERY-000 Verification 001

## 范围

本次验证覆盖 M2 Commit/Index 最小启动恢复完整链路：持久恢复身份、Journal v2/v1 安全迁移、真实 HEAD/Index 观测、desktop 启动门禁及跨进程会话重新绑定。恢复过程不执行 Git 写操作、不自动重试，也不覆盖真实 Index。

## 验证结果

`RecoveryScope` 独立于随机 `SessionTarget`，由 Git discovery 返回的规范化 common dir 与具体 git dir 构造；Unix 路径保留原始字节，Windows 路径保留 UTF-16 code units。真实 linked worktree 测试证明共享 common dir 时仍因 git dir 不同而获得不同 scope。

Journal schema v2 要求新记录携带 scope。存储适配器仍可读取 v1：已完成记录保留但不进入恢复，未完成记录作为 orphaned 返回；v2 缺 scope、未知 schema 和截断 JSON 均 fail-closed。追加 v2 不删除 v1 数据。

`StartupRecoveryInspector` 按 scope 读取 pending Journal，只进行一次新的 HEAD/Index 观测，并把旧 target 的记录绑定到当前 session target。`classify_recovery` 输出四种安全状态：

- `NotExecuted`：HEAD 与 Index 均保持期望值；
- `Committed`：Journal 记录的提交已成为 HEAD，Index 也符合期望；
- `CommittedNeedsReconcile`：提交已成为 HEAD，但 Index 需要后续协调；
- `Unknown`：无法证明属于以上状态，禁止自动继续。

desktop repository bridge 在任何写 dispatcher 安装前执行检查。Journal 路径缺失、读取失败、orphaned、`Unknown` 或 `CommittedNeedsReconcile` 时显示“写操作已禁用”，但 Status/Log 只读刷新继续工作。

已执行并通过：

```text
cargo fmt --all
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
git diff --check
```

真实仓库测试覆盖：销毁第一套 Registry 后由第二套随机 target 找回旧 Journal；原 HEAD/Index 判 `NotExecuted`；外部 Stage 后判 `Unknown` 并阻止写入；损坏 Journal 阻止写入但不停止只读刷新；生产 Stage executor 的 Completed Journal 保留写前 Index 指纹。Rust CI #60（Runner ID `42367271332`）已验证 scope、Journal v2 与 desktop gate 的前一检查点；最终 Index 指纹与真实 linked-worktree 隔离仍需下一次三平台运行确认。

## 风险与后续

`NotExecuted` 与 `Committed` 当前是可证明状态，不自动重放或修改 Journal；后续恢复 UI 可提供显式确认/清理。`CommittedNeedsReconcile`、`Unknown`、orphaned 或损坏 Journal 保持人工检查，不做猜测性修复。Branch refs 的高级自动恢复不属于 Commit/Index 最小 `RECOVERY-000`；Branch 写入异常仍安全进入未知/只读门禁，不自动重试。
