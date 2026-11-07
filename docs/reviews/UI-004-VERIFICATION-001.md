# UI-004-VERIFICATION-001：Repository Tabs / StatusBar 验证记录

- 日期：2026-11-07
- 任务：UI-004
- 当前状态：本机 Workspace 全量门禁与 Rust CI #20 三平台门禁均通过

## 1. 范围与架构边界

本任务实现 IP-001 §4.1 UI-004：从 Core 已发布的 Session/Snapshot 投影当前 Repository Tab 与 StatusBar，并在 desktop composition root 打通一次真实只读 Git 加载。

依赖方向保持为 `Shell → git-ui-model → git-application/domain`。`idea-shell` 不引用 `git-cli`；Git CLI adapter 仅由 desktop 装配。UI 更新回到 GPUI App 上下文，后台 Git 读取不阻塞 UI 线程。

## 2. 投影语义

`RepositoryChromeModel::single_session` 使用 `RepositorySessionView` 的规范化位置生成 UI 展示名，使用 `RefreshState` 生成：

- 稳定 `SessionTarget`、active repository tab、仓库/工作树名称。
- Attached/Unborn branch 或 Detached HEAD 的分支文字。
- `ahead/behind` 同时可用时显示 `↑N ↓N`，未知 tracking 不编造零值。
- Snapshot revision、Pending/Ready/Degraded health。
- Pending 显示 `Refreshing…`，Degraded 显示 `Refresh failed`，不会显示 Ready。

Shell 的 `WorkbenchController` 是唯一跨异步边界的 UI 写入口；controller 接收纯投影而非 Git adapter 或可变 Snapshot。

## 3. Desktop 装配

设置显式 `IDEAGIT_OPEN_REPOSITORY=/path/to/repository` 时，desktop 后台线程创建 Tokio runtime，顺序执行：

```text
Repository Discovery → Registry.open → Session view
→ HEAD/Status/Tracking/HEAD guarded refresh → RepositoryChromeModel → GPUI controller
```

默认不读取当前目录或用户最近仓库，避免未经请求打开仓库。此入口是面向后续 UI/端到端自动化的受控打开路径，不替代正式 Open Repository Action。

## 4. 当前平台实际验证

当前环境：macOS 15.8 arm64、Rust/Cargo 1.99.1。

| 验证 | 结果 |
| --- | --- |
| UI projection 单元测试 | 通过：Attached branch/sync/revision、Pending/Degraded 失败路径 |
| 真实 Git fixture 集成测试 | 通过：创建隔离 `main` commit 后 Discovery、Session、Snapshot 与 `worktree/main/Ready` 投影一致 |
| `IDEAGIT_OPEN_REPOSITORY="$PWD" IDEAGIT_UI_SMOKE_DURATION_MS=1500 cargo run -p desktop --locked` | 通过：真实工作区加载、窗口绘制与自动关闭 |
| `cargo test -p git-ui-model -p idea-shell -p desktop --locked` | 通过；9 个测试 |
| `cargo clippy -p git-ui-model -p idea-shell -p desktop --all-targets --locked -- -D warnings` | 通过 |
| Workspace fmt/check/Clippy/tests/rustdoc | 通过 |

## 5. 遗留项与风险

- 当前只投影一个显式打开的 repository；多 repository tab 生命周期、切换/关闭由后续 UI 工作补齐。
- 这一次刷新不持有持续 Scheduler/Watcher；UI-005 与 §4.7 E2E 会将 CORE-008 的 Watcher/Refresh 持续绑定到打开会话。
- 没有实现 Stage/Unstage 或 Commit，不能以本任务声称 M2 写操作就绪。
- [Rust CI #20](https://github.com/dengxiujun/IdeaGit/actions/runs/42343244836) 已在 Windows Server 2022、macOS 15 与 Ubuntu 24.04 完成 Format、locked check、Clippy 与 Workspace tests；真实多显示器、IME、DPI 仍属于平台 QA。
