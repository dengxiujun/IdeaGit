# CORE-005-VERIFICATION-001：HEAD / Branch Reader 验证记录

- 日期：2026-11-07
- 任务：CORE-005
- 当前状态：完成；当前 macOS 与三平台 CI 均通过

## 1. 范围与契约

本任务实现 Git Core §18.2/18.3 定义的 `HeadState`、HEAD 读取端口，以及当前分支的 upstream/ahead/behind 读取。不实现 Snapshot Builder、Repository Session、Watcher、分支写操作、Log 或 UI。

`HeadState` 保持批准的三个互斥状态：

- `Unborn { branch }`
- `Attached { branch, oid }`
- `Detached { oid }`

当前分支跟踪状态单独建模为 `BranchTracking { upstream, ahead, behind }`，没有修改或复制已批准的 `HeadState`。`HeadReader` 与 `BranchReader` 接收统一的 `RepositoryLocation` 和 `ExecutionContext`，生产适配器为 `GitCliHeadReader`。

## 2. Git CLI 实现

实现使用稳定的 Git plumbing 命令，不经 Shell 拼接：

- `symbolic-ref --quiet [--short] HEAD` 区分 symbolic、detached 与异常状态。
- `rev-parse --verify HEAD` 读取完整 SHA-1/SHA-256 OID，并与 symbolic 结果组合出三个 `HeadState`。
- `for-each-ref --format=%(upstream)` 读取完整 upstream ref；未配置 upstream 返回 `None`。
- `rev-list --left-right --count HEAD...<upstream>` 计算 ahead/behind。

所有命令均设置 `--no-optional-locks`、禁用终端提示、复用有界且可取消的 `GitCliExecutor`。Ref 保持原始字节并经 `RefName` 校验，OID 经 `ObjectId` 校验；非法或多记录输出不会被静默接受。Bare repository 使用已发现的 `git_dir` 作为工作目录，让 Git 自行识别 bare repository，不伪造 worktree，也避免把 Windows verbatim path 重复注入 `GIT_DIR`。

## 3. 当前平台实际验证

当前环境：macOS 15.8 arm64、Rust/Cargo 1.99.1、Apple Git 2.39.5。

真实隔离 Git Fixture 覆盖：

1. Unborn、attached 与 detached HEAD。
2. 未配置 upstream 与 detached HEAD 的 `None` 结果。
3. 本地 bare remote 的初始 0/0 tracking。
4. 本地与 peer 分别提交后的真实 1 ahead / 1 behind 分叉。
5. Bare repository 的 symbolic HEAD。
6. 非法 ahead/behind 输出拒绝。

Fixture 显式使用 `main` 作为初始分支，并把 `init.defaultBranch` 写入隔离的临时全局配置，测试不再依赖用户或 Runner 的默认分支设置。

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo test -p git-domain -p git-ports -p git-cli -p git-fixture --locked` | 通过；53 个测试全部成功 |
| `cargo check --workspace --all-targets --locked` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 |
| `cargo test --workspace --locked` | 通过；62 个 Workspace 测试全部成功 |
| `RUSTDOCFLAGS='-D warnings' cargo doc -p git-domain -p git-ports -p git-cli --no-deps --locked` | 通过；公共 API 文档无警告 |

## 4. 三平台 CI 验证

[Rust CI #14](https://github.com/dengxiujun/IdeaGit/actions/runs/42319097515) attempt 2 已完成并全部通过：

- [Windows Server 2022](https://github.com/dengxiujun/IdeaGit/actions/runs/42319097515/job/128245871924)
- [Ubuntu 24.04](https://github.com/dengxiujun/IdeaGit/actions/runs/42319097515/job/128245871897)
- [macOS 15](https://github.com/dengxiujun/IdeaGit/actions/runs/42319097515/job/128245871919)
- [Format](https://github.com/dengxiujun/IdeaGit/actions/runs/42319097515/job/128245871680)

首次代码运行 [Rust CI #13](https://github.com/dengxiujun/IdeaGit/actions/runs/42317764245) 暴露 Windows Git 不能把 `canonicalize` 产生的 verbatim path 作为 `GIT_DIR` 使用；修正为以 bare `git_dir` 作为 cwd 后，Windows 测试通过。Rust CI #14 attempt 1 的 Windows/macOS 已成功，但 Ubuntu apt 安装约 20 分钟无进展，因此取消该次基础设施卡住的尝试；attempt 2 重新执行全部平台并全部通过，没有用部分成功替代最终门禁。

## 5. 依赖、设计差异与遗留事项

- 无新增第三方依赖，Cargo.toml 与 Cargo.lock 不应改变。
- 没有复用 status header 作为 HEAD 的权威来源；HEAD/Branch ports 直接读取 Git ref，Snapshot Builder 后续再组合各 reader 的结果。
- Upstream 在 detached 或未配置时返回 `None`；读取计数期间 ref 变化导致命令失败时归类为 `ReferenceChanged`，不自动重试或伪造旧计数。
- 当前仅验证本地 ref/upstream；远程网络状态、gone upstream、replace refs、超大提交图性能与外部进程在多条 plumbing 命令之间并发改写 ref 尚未覆盖。
- Windows、macOS、Linux 的命令输出、bare HEAD、upstream 与分叉计数已由 Rust CI #14 attempt 2 验证。Windows verbatim path 不再重复注入 `GIT_DIR`。
