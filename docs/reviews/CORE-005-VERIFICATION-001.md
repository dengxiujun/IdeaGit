# CORE-005-VERIFICATION-001：HEAD / Branch Reader 验证记录

- 日期：2026-11-07
- 任务：CORE-005
- 当前状态：当前 macOS 实现及检查通过；本提交三平台 CI 待执行

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

所有命令均设置 `--no-optional-locks`、禁用终端提示、复用有界且可取消的 `GitCliExecutor`。Ref 保持原始字节并经 `RefName` 校验，OID 经 `ObjectId` 校验；非法或多记录输出不会被静默接受。Bare repository 使用已发现的 `git_dir` 作为 `GIT_DIR`，不伪造 worktree。

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

## 4. 依赖、设计差异与遗留事项

- 无新增第三方依赖，Cargo.toml 与 Cargo.lock 不应改变。
- 没有复用 status header 作为 HEAD 的权威来源；HEAD/Branch ports 直接读取 Git ref，Snapshot Builder 后续再组合各 reader 的结果。
- Upstream 在 detached 或未配置时返回 `None`；读取计数期间 ref 变化导致命令失败时归类为 `ReferenceChanged`，不自动重试或伪造旧计数。
- 当前仅验证本地 ref/upstream；远程网络状态、gone upstream、replace refs、超大提交图性能与外部进程在多条 plumbing 命令之间并发改写 ref 尚未覆盖。
- Windows/Linux 的命令输出、默认 Git 版本与 bare/upstream 行为必须等待本提交 CI；CI 通过前不得声称三平台验证完成。
