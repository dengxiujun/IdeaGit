# CORE-003-VERIFICATION-001：ProcessRunner / GitCliExecutor 验证记录

- 日期：2026-11-07
- 任务：CORE-003
- 当前状态：已完成；当前 macOS 检查及 Rust CI #10 三平台验证全部通过

## 1. 范围与边界

本任务只实现通用进程契约、Tokio 进程执行器和受封闭类型约束的 Git CLI 入口，不实现 Repository Discovery、Status/HEAD 解析、Snapshot、Watcher、Git 写操作或 UI。

`git-ports` 统一定义 `ExecutionContext`、`ProcessSpec`、环境覆盖、stdin、输出、进度及 `ProcessRunner`；`git-cli` 实现无 Shell 的 `TokioProcessRunner` 与 `GitCliExecutor`。Git 命令 trait 被 sealed，外部调用者只能执行适配器公开的强类型命令，不能传入任意参数字符串；本轮只公开无仓库副作用的 `GitVersionCommand` 探针。

非零退出码保留为 `ProcessOutput`，由后续强类型 Git adapter 根据具体命令解释，执行器不把退出码猜成业务状态。

## 2. 执行与安全设计

- executable、cwd、`Vec<OsString>` 参数和环境变量分别交给 `tokio::process::Command`，不经过 Shell。
- stdout/stderr 同时消费，每个流独立强制字节上限；超限立即终止并回收进程树。
- `ProcessStdin` 支持空、受控字节和有界 channel 流；子进程先退出时不会等待仍开放的输入生产者。
- `CancellationToken`、命令 timeout 与 `ExecutionContext.deadline` 共同生效，取最早截止时间。
- Unix 使用新进程组，Windows 使用 Job Object；超时、取消及管道失败均调用进程树 kill + wait，而不是把取消 future 当成终止证明。
- `ProcessSpec`、环境值、stdin 与输出的 Debug 不暴露参数、路径、凭据或原始输出；IO 诊断只记录错误类别和 OS code。
- 进度 channel 有界并以 `try_send` 投递无内容事件；消费者变慢时允许丢弃中间字节计数，不阻塞子进程管道或 UI 线程。

## 3. 新增依赖依据

新增并精确锁定 `process-wrap 10.0.1`，Rust version 要求为 1.87，低于项目 Rust 1.99.1 基线。只启用以下特性：

| Feature | 用途 |
| --- | --- |
| `tokio1` | 包装现有 Tokio Command/Child |
| `process-group` | Unix 建立、终止并等待独立进程组 |
| `job-object` | Windows 创建并终止 Job Object |
| `kill-on-drop` | 异常 future drop 时保留底层兜底终止语义 |

关闭默认的 tracing、process-session 和额外 creation-flags 特性。Cargo.lock 只新增 `process-wrap 10.0.1` 与其此前未解析的 `nix 0.31.3`；未升级或降级既有依赖。直接使用 Tokio `Child::kill` 只能保证顶层进程，无法满足 CORE-003 的 Windows Job、Unix 进程组及嵌套进程清理验收，因此该依赖是本任务的最小平台修正，不改变核心技术栈或模块边界，无需 ADR。

## 4. 当前平台实际验证

当前环境：macOS 15.8 arm64、Rust/Cargo 1.99.1、Apple Git 2.39.5。

`git-cli` 新增 13 个测试，`git-ports` 新增 1 个测试，覆盖：

1. 真实 `git --version` 强类型命令及自定义 executable 注入。
2. 流式 stdin、子进程提前退出且 producer 保持开放。
3. 非 UTF-8 原始输出与 stdout/stderr 同时大输出。
4. 非零退出码 23、启动失败和脱敏诊断。
5. stdout/stderr 超限、timeout 与显式 cancellation。
6. 父进程创建后代后超时，等待超过后代预定写入时间仍无 marker 文件，证明当前平台进程组已清理。
7. `ProcessSpec`、环境、stdin 和输出 Debug 不包含秘密内容。

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo check -p git-ports -p git-cli --all-targets --locked` | 通过 |
| `cargo clippy -p git-ports -p git-cli --all-targets --locked -- -D warnings` | 通过 |
| `cargo test -p git-ports -p git-cli --locked` | 通过；14 个测试全部成功 |
| `cargo metadata --no-deps --format-version 1 --locked` | 通过；锁文件可重现解析 |
| `cargo check --workspace --all-targets --locked` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 |
| `cargo test --workspace --locked` | 通过；40 个 Workspace 测试全部成功 |
| `RUSTDOCFLAGS='-D warnings' cargo doc -p git-ports -p git-cli --no-deps --locked` | 通过；公共 API 文档无警告 |

## 5. 三平台验证

[Rust CI #10](https://github.com/dengxiujun/IdeaGit/actions/runs/42279427776) 对提交 `0891ae8` 执行固定 Rust 1.99.1 的 locked metadata、check、Clippy、Workspace tests 和窗口烟雾测试，结果全部成功。CORE-003 的超时、取消、输出限制与嵌套进程 marker 测试在三种 Runner 上实际执行，因而 Windows Job Object 与 Linux/macOS process group 均有平台运行证据。

| Job | 结果 | 证据 |
| --- | --- | --- |
| Format | 通过 | [Job](https://github.com/dengxiujun/IdeaGit/actions/runs/42279427776/job/128104201815) |
| Windows Server 2022 | 通过 | [Job](https://github.com/dengxiujun/IdeaGit/actions/runs/42279427776/job/128104202506) |
| macOS 15 | 通过 | [Job](https://github.com/dengxiujun/IdeaGit/actions/runs/42279427776/job/128104202392) |
| Ubuntu 24.04 | 通过 | [Job](https://github.com/dengxiujun/IdeaGit/actions/runs/42279427776/job/128104202358) |

## 6. 设计差异、风险与遗留事项

- 计划没有指定跨平台进程树实现库；采用精确版本、最小 features 的 `process-wrap` 实现已要求的平台原语，并保留项目无 unsafe Rust。Windows Job 与 Unix process group 已由 CI #10 实际执行测试，但真实用户终端、权限受限 Job 和发行版差异仍属于后续平台验证。
- 进度事件只报告 PID、流和字节数，不复制可能含凭据的输出内容。后续确需展示 Git 进度时，应由具体命令 parser 生成脱敏的领域进度，而不是扩大此底层事件。
- 取消只证明外部进程树已终止，不证明 Git 仓库回滚；任何可能写仓库的后续调用仍必须经 OperationCoordinator 并执行 Reconcile。
- 本轮没有实现凭据 broker、TTY、交互提示或网络 Git。`GIT_TERMINAL_PROMPT` 等非交互策略由后续具体命令明确设置，不能由通用 runner 猜测。
