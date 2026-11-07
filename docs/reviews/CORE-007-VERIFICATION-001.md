# CORE-007-VERIFICATION-001：Repository Session / Registry 验证记录

- 日期：2026-11-07
- 任务：CORE-007
- 当前状态：当前 macOS 实现及 Workspace 全量门禁通过；本提交三平台 CI 待执行

## 1. 范围与权威契约

本任务实现 Git Core §6、§18.2、§18.3 与 IP-001 §4.5 中 Repository Session / Registry 的生命周期基础。不实现 CORE-008 Watcher/Refresh Scheduler，也不提前实现 Snapshot Store、Application Facade、事件订阅或 UI。

Registry 接受 CORE-002 已规范化的 `RepositoryLocation`。它以 `common_dir + worktree_root` 定位一个具体 Session，以 `common_dir` 分配逻辑 `RepositoryId`：

- 同一 worktree 的多个打开方共享 Session、`SessionTarget` 与世代。
- linked worktree 共享 `RepositoryId`，但拥有独立 `WorktreeId` 和 Session。
- bare repository 的 `SessionTarget.worktree` 保持 `None`。
- 不同 `common_dir` 的 Repository 与 Session 相互隔离。

## 2. 生命周期和过期保护

每次 `open` 返回不可克隆的 `RepositorySessionHandle`，表达一个调用方的所有权。显式 `close` 或 Drop 只释放该调用方；最后一个句柄释放后，Registry 删除 Session 并取消该世代的 `CancellationToken`。

Session generation 使用 Registry 内单调计数器，关闭后重新打开会分配更大的世代，旧 `SessionTarget` 查询返回 `SessionNotActive`。计数器和句柄数均使用 checked arithmetic；Mutex poison、外来 Registry handle、过期 target 和计数耗尽均使用结构化错误，不在生产路径使用 `unwrap()` 或 `expect()`。

CORE-008 可从 Registry 获取当前世代的取消令牌并派生 Watcher/Refresh 子任务；本任务未创建后台任务，因此没有伪造“Watcher 已取消”的验收结论。

## 3. 测试覆盖

`git-application` 单元测试覆盖：

1. 同一 worktree 重复打开复用 Session，逐个关闭保持引用计数，最后关闭失效。
2. linked worktree 共享 Repository 身份但隔离 Worktree 与 Session。
3. 不同仓库身份隔离，bare Session 不暴露虚假 worktree。
4. 最后关闭取消任务令牌，重新打开分配新世代并拒绝旧 target。
5. Drop 自动释放句柄，错误 Registry 拒绝外来 handle。
6. Session view 保留 Discovery 输出的 worktree、git dir 和 common dir。

## 4. 当前平台实际验证

当前环境：macOS 15.8 arm64、Rust/Cargo 1.99.1。

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo test -p git-application --locked` | 通过；6 个测试全部成功 |
| `cargo clippy -p git-application --all-targets --locked -- -D warnings` | 通过 |
| `cargo check --workspace --locked` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 |
| `cargo test --workspace --locked` | 通过 |
| `RUSTDOCFLAGS='-D warnings' cargo doc -p git-application --no-deps --locked` | 通过 |

## 5. 依赖、跨平台与遗留事项

- 只给 `git-application` 声明 Workspace 中已锁定的 `git-domain`、`thiserror` 和 `tokio-util` 直接依赖；没有新增第三方包或更改版本。Cargo.lock 仅同步本地包依赖清单。
- Session key 使用 Discovery 产出的 OS 原生 `PathBuf`，不把展示字符串或手工大小写转换当作文件系统身份；Windows/macOS/Linux 的真实 Discovery 集成由已有 CORE-002 覆盖，本提交仍需三平台编译测试。
- 当前引用计数和 Registry 状态使用短临界区同步 Mutex，不执行 IO，也不阻塞 GPUI UI 线程等待 Git；未来 IO 仍须在 Application Runtime/Git Worker 执行。
- 外部 Git 变化、Watcher 溢出、debounce、补刷新、Snapshot 发布和事件 lag 属于 CORE-008；多窗口 UI 焦点与请求序号属于 UI-004/005。
