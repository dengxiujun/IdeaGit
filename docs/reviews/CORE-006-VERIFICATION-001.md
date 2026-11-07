# CORE-006-VERIFICATION-001：Snapshot Builder 验证记录

- 日期：2026-11-07
- 任务：CORE-006
- 当前状态：当前 macOS 实现及 Workspace 全量门禁通过；本提交三平台 CI 待执行

## 1. 范围与权威契约

本任务实现 Git Core §18.2 与已批准 ADR D2 定义的唯一 `RepositorySnapshot`、`FileChange` 和 Snapshot Builder。不实现 Repository Session、Registry、Watcher、刷新调度、自动重试、Application Facade 或 UI。

IP-001 的 M1 任务表定义 CORE-006 依赖 CORE-004/005，验收为“同代版本、过期校验”，但正文没有单独的 CORE-006 小节；本实现因此以 Git Core §18 和 ADR D2 的已批准模型为权威，没有自行调整模块边界。

`RepositorySnapshot` 位于纯 `git-domain`，包含：

- `repository`、可选 `worktree`、单调 `revision`。
- `head`、可选触发 `operation`。
- `changes`、独立 `untracked`、可选 `ahead`/`behind`。
- `worktree_state`，明确区分有工作区状态与 bare 无工作区能力。
- `consistency`，明确区分一致读取与 stale 读取。

## 2. Builder 与一致性语义

`SnapshotBuilder` 只进行纯数据构造，不执行 IO。CORE-007/008 后续按以下顺序收集数据并调用它：

```text
HEAD guard(before) → porcelain v2 status → branch tracking → HEAD guard(after)
```

Builder 从当前 `SnapshotRevision` 使用 `checked_next` 分配整个结果唯一的下一代 revision；溢出返回结构化 `SnapshotBuildError`，不回绕。它按顺序检查：

1. 两次 `HeadState` 是否相同。
2. Porcelain `branch.oid` / `branch.head` 是否与最终 HEAD 一致，包括 Unborn、Attached 与 Detached。
3. Status 的 upstream 存在性及 ahead/behind 是否与独立 Branch Reader 一致。

任何校验不一致都会生成带 `SnapshotStaleReason` 的显式 stale Snapshot，不把混合读取声称为原子状态，也不在 Domain 内自动重试。后续 Session/Refresh Scheduler 可以识别 stale 并决定重新读取；stale Snapshot 不得授权写操作。

## 3. Status 投影与数据保真

- Ordinary、Rename/Copy 与 Unmerged 转成同一 `FileChange`，保持 Git 输出顺序。
- 保留 raw `RepoPath`、original path、Index/Worktree 双状态、HEAD/Index OID、三个 mode、Submodule 位和 similarity。
- Conflict 另外保留 kind、stage 1/2/3 mode 与 OID；七种 unmerged XY 都映射为对应的双状态，不压成展示字符串。
- Untracked 保存在独立集合；Ignored 不属于批准的 Snapshot 合约，因此不进入 Snapshot。
- Bare Snapshot 必须使用 `SnapshotRead::bare`，`worktree_state = Unavailable`；其空 changes/untracked 不能解释为“干净工作区”。Worktree 与 bare 输入混用会被拒绝。

## 4. 当前平台实际验证

当前环境：macOS 15.8 arm64、Rust/Cargo 1.99.1、Apple Git 2.39.5。

领域测试覆盖：

1. Ordinary、Rename、Unmerged、Untracked、Ignored 的完整投影。
2. 七种 unmerged XY 及三个 conflict stage。
3. HEAD guard 变化、status HEAD 不一致与 tracking 不一致三类 stale。
4. Unborn、Detached、bare worktree capability。
5. Worktree/bare 输入类型错误、revision 溢出。
6. Snapshot 序列化往返与非 UTF-8 raw RepoPath。

真实隔离 Git Fixture 集成测试覆盖：

1. Git CLI Repository/HEAD/Status/Branch Reader 输出构建一致 Snapshot。
2. 两次 HEAD guard 之间发生真实外部 commit 时生成 `HeadChangedDuringRead` stale Snapshot。

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo test -p git-domain -p git-cli --locked` | 通过；53 个单元测试与 2 个真实 Snapshot 集成测试全部成功 |
| `cargo clippy -p git-domain -p git-cli --all-targets --locked -- -D warnings` | 通过 |
| `cargo check --workspace --all-targets --locked` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 |
| `cargo test --workspace --locked` | 通过；71 个 Workspace 测试全部成功 |
| `RUSTDOCFLAGS='-D warnings' cargo doc -p git-domain --no-deps --locked` | 通过；公共 API 文档无警告 |

## 5. 依赖、设计差异与遗留事项

- 无新增第三方依赖，Cargo.toml 与 Cargo.lock 不应改变；`git-domain` 仍不依赖 Tokio、GPUI 或 IO 实现。
- `operation` 实现为当前已有强类型 `Option<OperationId>`；完整 Operation lifecycle/snapshot 属于 CORE-010，不在本任务提前定义第二套模型。
- Upstream ref 在 Status 中是展示用短名、Branch Reader 中是完整 ref；一致性检查比较 upstream 是否存在及精确 ahead/behind，不把短名文本猜测转换为完整 ref。
- Builder 只标记 stale，不负责读取、自动重试或发布；这些生命周期属于 CORE-007 Session 与 CORE-008 Refresh Scheduler。
- 当前未验证外部 ref 在每一条子命令之间的所有竞争窗口、超大 status 投影性能、多窗口发布、Watcher 丢事件或 UI 过期请求丢弃；这些继续由后续 M1 任务覆盖。
- Windows/Linux 的序列化、真实 Git 集成和 Snapshot 构建必须等待本提交 CI；CI 通过前不得声称三平台验证完成。
