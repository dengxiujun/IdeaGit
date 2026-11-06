## 关联任务与设计依据

- 任务 ID：
- 里程碑：
- 关联 Issue：
- 架构 / LLD / ADR：

## 变更范围

说明本 PR 实现的内容，以及明确不在本次范围内的内容。

## 实现与接口变更

- 主要实现：
- 公共 API / 数据模型 / 模块边界变化：
- 用户可见行为变化：

## 架构与依赖检查

- [ ] 依赖方向符合 `UI → Application → Domain / Ports`，适配器不反向依赖 UI。
- [ ] UI 未直接调用 Git CLI，Git 写操作仍通过 OperationCoordinator。
- [ ] 未引入 Crate 循环依赖。
- [ ] 新增或升级依赖已说明原因并更新 Cargo.lock；不适用时注明。
- [ ] 涉及核心版本、模块边界、核心数据模型或已批准设计时，已关联获批 ADR；不适用时注明。

## 验证结果

填写实际执行结果；未执行的检查必须写明原因，不得标记为通过。

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo check --workspace --locked`
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings`
- [ ] `cargo test --workspace --locked`
- 其他测试 / Fixture / E2E：

## 跨平台影响

| 平台 | 实际验证结果 / 未验证原因 |
| --- | --- |
| Windows |  |
| macOS |  |
| Linux |  |

## Git 数据安全与恢复

- 是否读取或写入真实 Git 仓库：
- 对 Worktree / Index / Refs / 用户配置的影响：
- 外部 Git 并发、失败、取消和恢复策略：
- 凭据、路径和诊断信息是否已脱敏：

## 风险与回滚

- 已知风险和遗留事项：
- 回滚方式：

## 提交前检查

- [ ] 仅包含当前任务范围内的修改。
- [ ] 公共 API 已添加 Rustdoc，相关文档和 PROGRESS.md 已按需更新。
- [ ] 未静默丢弃、覆盖或重置用户修改。
- [ ] 未执行的测试和平台已明确标注。
