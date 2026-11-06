# TEST-FIXTURE-VERIFICATION-001：TEST-001 Git Fixture Harness 验证记录

- 日期：2026-11-07
- 任务：TEST-001 Git Fixture Harness
- 当前状态：当前 macOS 平台验证通过；Windows、macOS、Linux 的本轮 CI 验证尚未执行

## 1. 范围

新增独立测试支持包 `tests/fixtures`（包名 `git-fixture`）。它只在测试中提供真实 Git 仓库夹具，不参与任何生产 crate 的依赖图，也不实现 Git Core、Git CLI Executor、UI 或真实网络远端功能。

`TestRepository::init()` 创建临时 worktree；`commit_file()` 写入、暂存指定相对路径并提交。夹具另提供本地 bare remote 的 Push/Pull 往返、三方冲突、linked worktree 和 Hook 安装场景，供后续 Git Contract 与集成测试复用。

## 2. 隔离设计

每个夹具都持有一个 `tempfile::TempDir`，所有 worktree、HOME、全局配置、Hooks、peer clone 和 bare remote 都位于该目录。所有由夹具启动的 Git 进程均使用下列环境，而不是用户的 Git 设置：

| 项目 | 隔离方式 |
| --- | --- |
| HOME / Windows 用户目录 | `HOME` 与 `USERPROFILE` 指向临时目录 |
| Git Config | `GIT_CONFIG_GLOBAL` 指向临时配置；`GIT_CONFIG_NOSYSTEM=1` 禁用系统配置 |
| Hooks | 临时全局 `core.hooksPath` 指向夹具目录 |
| Credential | 临时全局 `credential.helper` 为空，且 `GIT_TERMINAL_PROMPT=0` |
| 签名 | 临时全局 `commit.gpgSign=false` |
| Remote | 仅用同一临时目录中的 bare repository；不访问网络 |

命令通过 `std::process::Command` 直接调用 `git`，没有 shell 拼接。文件路径必须是相对路径且不含父目录组件，避免测试写出 fixture worktree。

## 3. 实际验证

环境：macOS 15.8（arm64）、Rust/Cargo 1.99.1、Apple Git 2.39.5。

`git-fixture` 的 5 个真实 Git 测试均通过：

1. 初始化空仓库并确认 Unborn HEAD、隔离的全局身份、签名和 credential helper。
2. 提交包含空格及换行的文件名。
3. 建立本地 bare `origin`，从 peer clone 提交并 Push，再由原 worktree fast-forward Pull。
4. 从两个分支修改同一路径，保留 `UU` 未解决冲突状态。
5. 创建 linked worktree；专用 `pre-commit` Hook 拒绝提交，且 HEAD 不改变。

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo check --workspace --all-targets --locked` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 |
| `cargo test -p git-fixture --locked` | 通过，5 个单元测试与 0 个文档测试 |
| `cargo test --workspace --locked` | 通过；新增 5 个 fixture 测试，既有测试亦通过 |

Cargo.lock 已随新 workspace 成员更新；没有新增第三方版本或传递依赖。

## 4. 尚未验证与边界

- 本提交尚未经 Windows、macOS、Linux 三平台 CI 运行；当前结果不能替代三平台验收。
- 夹具隔离了签名和凭据的默认配置，但没有调用真实 GPG/SSH 签名代理或 credential helper；这些平台差异及失败注入留给相关后续写操作任务。
- 未覆盖非 UTF-8 路径、SHA-256 仓库、Index.lock、网络中断、外部 Git 竞争、rebase conflict 或恢复流程。
- 本任务不开放任何生产写操作，也不证明 OperationCoordinator、Lock、Journal、Index 安全协议或高级 Partial Commit 的正确性。
