# CORE-009-VERIFICATION-001：强类型 GitCommand 验证记录

- 日期：2026-11-07
- 任务：CORE-009
- 当前状态：本机专项通过；本提交三平台 CI 待执行。

## 1. 范围与契约

本任务实现 IP-001 §5.1 的 CORE-009 及 LLD-GIT-CORE-001 §18.4 命令契约。所有命令位于纯 `git-domain`，不依赖 Tokio、GPUI、Git CLI 或操作协调器；命令只表达意图，不执行外部副作用。

覆盖：

- `StageFiles` / `UnstageFiles`；
- `StageSelection` / `UnstageSelection`，绑定 `FileIdentity` 与 `SelectionFingerprint`；
- `ApplyEditedIndex`、`CommitIndex`、`CommitSelection`；
- `Branch`、`Remote`、`Sequencer` typed intent。

每个 `GitCommand` 变体只携带一个 `SessionTarget`，嵌套 selection 的目标必须与外层一致。Branch/Remote 使用 `RefName`，文件操作使用经验证的 `RepoPath`。

## 2. 安全校验

`GitCommand::validate` 拒绝：

- 空 Stage/Unstage 路径；
- 空 Commit message；
- 空 selection；
- 外层与嵌套 `SessionTarget` 不一致；
- selection 与 fingerprint 数量不匹配。

本任务没有实现 Git 执行、真实 Index 锁、Operation、Journal、自动重试或写入恢复；这些由 CORE-010/011/012 与 GIT-001/002 在前置门禁后实现。

## 3. 已执行验证

- `cargo fmt --all -- --check`
- `cargo check -p git-domain --locked`
- `cargo clippy -p git-domain --all-targets --locked -- -D warnings`
- `cargo test -p git-domain --locked`：23 个测试通过。
- 测试覆盖单目标提取、跨会话选择拒绝、空消息/空选择拒绝；未触碰真实用户仓库。

## 4. 遗留风险

- 本提交尚未执行 Workspace 全量门禁和三平台 Runner；推送后需按 2 分钟间隔检查。
- 命令类型尚未被 Operation Coordinator 消费；在 CORE-010 完成前不能声称 Stage/Commit 已可执行。
