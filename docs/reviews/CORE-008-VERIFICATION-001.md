# CORE-008-VERIFICATION-001：Watcher + Refresh Scheduler 验证记录

- 日期：2026-11-07
- 任务：CORE-008
- 当前状态：当前 macOS 实现及 Workspace 全量门禁通过；本提交三平台 CI 待执行

## 1. 范围与权威契约

本任务实现 Git Core §6 与 IP-001 §4.1 的 CORE-008：Watcher 只产生“可能变化”信号，Application Refresh Scheduler 读取权威 Git 状态、合并事件、周期补刷新并发布当前不可变 Snapshot。

当前共享契约尚未定义 Sequencer probe port；本任务没有复制第二套 Sequencer 模型或修改模块边界。Sequencer/Operation 状态按后续已跟踪任务实现。本任务不包含 UI、写操作或布局。

## 2. Watcher 与路径范围

`RepositoryWatch` 使用 `notify::recommended_watcher` 和有界 Tokio channel：

- 普通仓库递归监控 worktree；因 `.git` 已包含其中，不重复注册 common dir。
- linked worktree 同时监控具体 worktree 与共享 common dir，以捕获工作区、Index 和公共 refs 变化。
- bare repository 只监控 common dir，不伪造 worktree。
- 后端错误转换为 `RescanRequired`；通道满时已有排队信号仍保证一次刷新，30 秒主动校验提供丢事件/溢出的最终兜底。

Watcher 不读取 Git、不构建 Snapshot，也不直接修改 Session 状态。

## 3. Guarded Refresh 与发布语义

`SnapshotRefreshService` 对 worktree 严格执行：

```text
HEAD(before) → porcelain v2 Status → Branch Tracking → HEAD(after) → SnapshotBuilder
```

bare 跳过 Status，使用 `SnapshotRead::bare`。所有读取共享 Session cancellation 的子令牌；失败不自动重试或猜测结果。

`RefreshScheduler`：

- 默认 200ms debounce 并清空同一批突发 trigger，确保单 Session 串行刷新。
- 默认每 30 秒主动验证，即使 Watcher 丢事件也会重新读取。
- 通过 Tokio `watch` 只发布当前 `RefreshState`，不是无界事件日志。
- 成功时发布单调 revision Snapshot；失败时进入 Degraded 并保留最后一个成功 Snapshot；Session cancellation 后退出。
- 配置拒绝零容量、零 debounce 或零验证周期。

## 4. 测试与当前平台验证

新增测试覆盖：

1. ordinary/linked/bare watch roots 与路径过滤。
2. 零容量在触碰文件系统前拒绝。
3. guarded refresh 的双 HEAD、一次 Status 和下一 revision。
4. 突发 filesystem/overflow 信号合并为一次刷新。
5. Session cancellation 停止调度器。
6. 读取失败进入 Degraded 且不伪造 Snapshot。
7. 非法调度配置结构化拒绝。

当前环境：macOS 15.8 arm64、Rust/Cargo 1.99.1。

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo test -p git-application -p repository-watcher --locked` | 通过；13 个测试全部成功 |
| `cargo clippy -p git-application -p repository-watcher --all-targets --locked -- -D warnings` | 通过 |
| `cargo check --workspace --locked` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 |
| `cargo test --workspace --locked` | 通过 |
| `RUSTDOCFLAGS='-D warnings' cargo doc -p git-application -p repository-watcher --no-deps --locked` | 通过 |

## 5. 依赖、跨平台与剩余风险

- 只声明 Workspace 已锁定的 Tokio、async-trait 和 thiserror 直接依赖；没有新增第三方包或版本变更。Cargo.lock 仅同步本地包依赖清单。
- `notify::recommended_watcher` 在 Windows/macOS/Linux 选择平台后端；本提交仍需三平台编译测试。真实桌面长时间运行、Watcher 后端溢出注入和网络文件系统行为尚未验证。
- Watcher 回调只执行路径过滤与 `try_send`，不阻塞后端线程；Git IO 全部在异步 Refresh task 中执行，不阻塞 GPUI UI 线程。
- 当前调度器提供独立构件，Desktop/Workbench 的端到端装配属于 UI-003/UI-004/UI-005 与 M1 §4.7 门禁。
