# UI-005-VERIFICATION-001：Read-only Changes Tree 验证记录

- 日期：2026-11-07
- 任务：UI-005
- 当前状态：本机专项与 Workspace 全量门禁通过；本提交三平台 CI 待执行。

## 1. 范围与架构边界

本任务实现 IP-001 §4.1 的 UI-005，并关闭 §4.7 所要求的 M1 只读链路：Discover、Session、Porcelain v2、Snapshot、Workbench Changes，以及外部 Git 修改经 Watcher/Refresh 后更新 UI。

`git-ui-model` 仅将不可变 `RepositorySnapshot` 投影为 ViewModel；`idea-shell` 只渲染投影；`desktop` 作为组合根持有 `git-cli`、`repository-watcher` 和 `RefreshScheduler`。UI crate 不依赖 Git CLI，所有 Git IO 均在后台 Tokio runtime。

## 2. Changes 投影语义

- `ChangesTreeModel` 固定包含 `Staged`、`Unstaged`、`Untracked` 三组，并保留 `SessionTarget`、Snapshot revision、原始 `RepoPath`、rename 原路径和冲突标记。
- Index 与 Worktree 是不同内容源。同一路径同时有已暂存和未暂存变化时，必须分别出现在前两组；不会以一个 Checkbox 或字符串状态覆盖另一侧。
- Untracked 保持 Snapshot 中的 Git 输出顺序，展示为 Worktree 的新增项；本任务没有 Stage、Unstage、Commit 或任何 Git 写入口。
- Commit Dock panel 显示三个只读组及文件状态。异步模型只经 `WorkbenchController` 回到 GPUI App 上下文更新 Entity。

## 3. 持续刷新与生命周期

- desktop 在发现路径后创建 Registry session，从该 session 取得 cancellation token，并启动现有 `RefreshScheduler` 与 `RepositoryWatch`。
- Watcher 的 `Changed`/`RescanRequired` 只请求权威刷新；刷新后通过 latest-only watch state 重新投影，不把文件系统事件当作 Git 真相。
- UI receiver 关闭时 bridge 取消 session，等待 scheduler 收束后释放 session handle；不会在 GPUI UI 线程执行 Git IO。

## 4. 已执行验证

- `cargo fmt --all -- --check`
- `cargo check --workspace --locked`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test --workspace --locked`
- 真实隔离 Git fixture：初始 Snapshot 同时包含 staged、unstaged、mixed 和 untracked；mixed 文件在两个对应组中出现。随后通过 fixture 在应用外写入 `external.txt`，实际 Watcher 信号触发 Scheduler 刷新，并验证新的 Workbench 模型出现该文件。
- 原生窗口 smoke：以 `IDEAGIT_OPEN_REPOSITORY` 打开真实仓库，并以 `IDEAGIT_UI_SMOKE_DURATION_MS` 绘制、关闭。

## 5. 遗留风险

- 三平台 CI 仍须验证本提交的 Watcher/桌面 bridge；本机成功不替代平台验收。
- M1 不开放写操作。Stage/Unstage、Commit、Action Dispatcher、Draft 与 Log 属于 M2 任务，不能从只读投影推断其已就绪。
- 真正的 Windows/Wayland 长时间 Watcher 行为、网络文件系统与多窗口多仓库交互仍属于后续平台 QA。
