# IdeaGit 开发进度

更新日期：2026-11-07。

## 当前状态

M0 已完成，M1 Git Read Core 与 Workbench 正在实施。CORE-001/002/003/004/005/006/007/008、UI-003 已完成当前平台及三平台 CI 验证；UI-004 Repository Tabs / StatusBar 已在当前 macOS 通过专项及 Workspace 全量门禁，等待本提交三平台 CI。Read-only Changes Tree 继续由 UI-005 实施。

## 已完成的文档工作

- 首轮审查：ARCH-REVIEW-001、API-CONFLICTS-001、M0-EXECUTION-CHECKLIST。
- 项目负责人已在聊天批准 [ADR-ARCH-REMEDIATION-001](../architecture/adr/ADR-ARCH-REMEDIATION-001.md) D1–D7，并要求先迁移 ADR 目录。
- 已将 docs/adr 移至 docs/architecture/adr，修正链接并记录 Accepted 状态。
- 根据 ADR 修订六份设计/基线/主计划：共享契约集中 Core §18；Index 写后恢复捷径移除；状态/恢复统一；任务前置和发布门禁补齐。
- M0 清单保留首轮发现，并补充设计修订与待验证事项的区分。
- INFRA-004 已由根 AGENTS.md、正式 ADR、[Workspace 依赖记录](INFRA-001-workspace.md) 和 GitHub PR 模板共同闭环。

## 架构修订阶段验证记录（历史）

本次检查结果：

- 相对 Markdown 文件链接检查通过（架构文档及主计划/M0 清单/进度记录）。
- 从修订后的 IP 实际文件提取 90 个任务：无重复 ID、缺失依赖引用或循环。
- git diff --check 通过；旧 docs/adr 目录已不存在，新目录包含正式 ADR。
- 已清理旧 SnapshotVersion、冲突共享 DTO 定义、真实 Index 写后恢复捷径及仅按 RepositoryId 定位的 UI 执行上下文。

以下限制描述的是 Workspace 建立前的架构修订阶段：当时未执行 Cargo 构建、Rust 测试、依赖安装、三平台窗口或真实 Git 故障注入。后续工程验证结果见 INFRA-001/002/003 各节。

## 未关闭的工程门禁

| 范围 | 状态 / 放行条件 |
|---|---|
| Rust/GPUI/其他依赖 | Rust 1.99.1 与锁定依赖已在三平台 CI 编译；MSRV、真实窗口和 Git 最低版本/能力矩阵待验证 |
| Workspace | 25 个包依赖无环；Rust CI #3 已在 Ubuntu 24.04、Windows Server 2022 和 macOS 15 通过 locked check、Clippy 与测试 |
| 标准写操作 | Lock/Journal/RECOVERY-000/真实 Git fixture 完成后才能开放 |
| Partial Commit / Index 编辑 | capability 默认关闭；锁内比较/发布、剩余 staged 转移、Hook/HEAD 竞争及 TEST-004 未验证 |
| UI | GPUI-Kit 最小窗口已在三平台 Runner 启动；Dock、输入、焦点、IME、DPI 和真实桌面兼容性仍未验证 |
| 发布 | M0–M6 均不能因文档完成而标记交付；最终仍需 QA、安全、迁移与阻断项清零 |

## 下一步

UI-004 三平台 CI 通过后，按依赖 DAG 实施 UI-005 Read-only Changes Tree；完成后执行 M1 §4.7 首条端到端门禁。后续仍一次只实施一个任务，核心版本或边界变更仍需 ADR。

## UI-004 Repository Tabs / StatusBar（2026-11-07）

- `git-ui-model` 新增从 `RepositorySessionView + RefreshState` 到 Repository Tab/StatusBar 的纯投影：仓库显示名、SessionTarget、当前分支、ahead/behind、Snapshot revision 与 Pending/Ready/Degraded 均保留且不猜测成功。
- Shell 增加 typed `WorkbenchController`；后台结果只通过 GPUI 调度更新窗口 Entity，UI crate 不依赖 `git-cli`。
- desktop composition root 支持 `IDEAGIT_OPEN_REPOSITORY` 打开路径：在后台 Tokio runtime 执行 CORE-002 Discovery、CORE-007 Session、CORE-008 guarded refresh，再投影到 Shell；该变量同时用于自动化 E2E，默认不擅自打开目录。
- 当前 macOS 已通过 2 个 projection 测试、真实隔离 Git fixture 集成测试、实际桌面窗口 smoke 与专项 Clippy；三平台 CI 待本提交推送后执行。
- UI-004 不实现多 tab 交互/关闭、持续 Watcher 订阅或 Changes Tree，分别留给后续 UI/M1 集成。详细结果见 [UI-004-VERIFICATION-001](../reviews/UI-004-VERIFICATION-001.md)。

## UI-003 Workbench Shell / DockArea（2026-11-07）

- `git-workbench` 新增窗口本地 `WorkbenchState`、稳定 `PanelKind`/PanelRegistry 名称、Overlay ID 和版本化 layout envelope；旧 schema、空 layout ID 与非法 overlay 均显式拒绝。
- `idea-shell` 将 UI-001 启动占位替换为 GPUI-Kit 0.9.0 `DockArea`：中心 Commit/Git Log/Diff 标签、左侧 Repository、底部 Git Console，并提供 Titlebar 区、Repository Tab 占位、StatusBar 与最后绘制的 OverlayHost。
- 所有面板经 PanelRegistry 注册，可拖拽、关闭、隐藏与恢复；未知/已移除面板按 GPUI-Kit DockArea 的占位语义安全保留，不在一次保存后静默丢失。
- DockArea `LayoutChanged` 以 250ms 世代防抖，只发布最终布局；desktop 在后台线程写入平台配置目录，启动前读取并验证 schema。UI 线程不执行文件 IO。
- 当前 macOS 已连续两次运行真实窗口烟雾：首次生成布局，第二次加载同一布局并成功绘制/关闭；专项 9 个测试、fmt 与 Clippy 通过。[Rust CI #19](https://github.com/dengxiujun/IdeaGit/actions/runs/42341427891) 已在 Windows Server 2022、macOS 15 和 Ubuntu 24.04 全部通过；UI-003 三平台验收关闭。
- UI-003 不伪造真实 Repository/HEAD/Changes；这些属于 UI-004/005。详细结果见 [UI-003-VERIFICATION-001](../reviews/UI-003-VERIFICATION-001.md)。

## CORE-008 Watcher + Refresh Scheduler（2026-11-07）

- `repository-watcher` 新增基于 `notify` 的有界、内容无关变化信号；普通仓库递归监控 worktree（包含 `.git`），linked worktree 同时监控独立 worktree 与共享 common dir，bare 只监控 common dir。
- `git-application` 新增 guarded Snapshot 刷新服务，严格按 `HEAD(before) → Status → Tracking → HEAD(after)` 读取并调用唯一 `SnapshotBuilder`；bare 不执行 worktree Status。
- Refresh Scheduler 合并 200ms 内突发信号，使用 30 秒周期主动校验兜底丢事件/溢出，串行发布单调 Snapshot；失败保留最后成功 Snapshot 并进入 Degraded，Session 取消时停止后台任务。
- 当前 macOS 已通过 fmt、locked check、Clippy 和 Workspace tests；新增 7 个 Watcher/Refresh 正常及失败路径测试。[Rust CI #18](https://github.com/dengxiujun/IdeaGit/actions/runs/42340135972) 已在 Windows Server 2022、macOS 15 和 Ubuntu 24.04 全部通过；CORE-008 三平台验收关闭。
- CORE-008 不提前实现 Sequencer port、Operation 写协调或 UI；详细结果见 [CORE-008-VERIFICATION-001](../reviews/CORE-008-VERIFICATION-001.md)。

## CORE-007 Repository Session / Registry（2026-11-07）

- `git-application` 新增线程安全 `RepositoryRegistry`、显式所有权 `RepositorySessionHandle` 和只读 `RepositorySessionView`；同一规范化 worktree 的多次打开共享同一 Session 与世代，最后一个句柄释放后才关闭。
- 同一 `common_dir` 的 linked worktree 共享 `RepositoryId`，但各自拥有 `WorktreeId`、Session 和生命周期；不同仓库保持身份及事件目标隔离，bare Session 不伪造工作区。
- Session 世代全局单调且关闭后不复用；旧 `SessionTarget` 被拒绝。每个 Session 提供取消令牌，最后关闭时取消，为 CORE-008 的 Watcher/Refresh 后台任务建立生命周期边界。
- 当前 macOS 已通过 fmt、locked check、Clippy 和 Workspace tests；`git-application` 新增 6 个正常/失败路径测试。[Rust CI #17](https://github.com/dengxiujun/IdeaGit/actions/runs/42339187095) 已在 Windows Server 2022、macOS 15 和 Ubuntu 24.04 全部通过；CORE-007 三平台验收关闭。
- CORE-007 不提前实现 Watcher、刷新调度、Snapshot Store、Git Application Facade 或 UI；详细结果见 [CORE-007-VERIFICATION-001](../reviews/CORE-007-VERIFICATION-001.md)。

## CORE-006 Snapshot Builder（2026-11-07）

- [Rust CI #16](https://github.com/dengxiujun/IdeaGit/actions/runs/42337894961) 已对提交 `d456896` 完成三平台门禁并成功；CORE-006 三平台验收关闭。

## INFRA-004 架构与开发约束（2026-11-07）

- 已批准的 [ADR-ARCH-REMEDIATION-001](../architecture/adr/ADR-ARCH-REMEDIATION-001.md) 固化 D1–D7 架构决策，受影响设计与主计划已完成同步。
- 根 AGENTS.md 已定义架构、Git 安全、Rust 质量、必需检查、完成定义和单任务范围约束。
- [INFRA-001 Workspace 记录](INFRA-001-workspace.md) 列出 25 个包的直接依赖；实际 Cargo 解析图已验证无环，UI 无直接 `git-cli` 依赖。
- 新增 `.github/PULL_REQUEST_TEMPLATE.md`，要求记录任务与设计依据、范围、接口、架构依赖、实际测试、三平台影响、Git 数据安全、风险和回滚。
- INFRA-004 已完成；本次收尾仅补 PR 模板与进度记录，没有修改架构决策、代码、依赖或 CI。

## INFRA-001 本机验证（2026-11-06）

- 根 Cargo.toml、Cargo.lock、25 个包的 Cargo.toml 与入口源码已建立；成员路径和依赖见 [Workspace 记录](INFRA-001-workspace.md)。
- 8 个 Core/Application、5 个 Adapters、10 个 UI、1 个 Platform 库及 desktop 二进制，统一 Edition 2024、Resolver 3，仅依赖本地包。
- 当前环境：macOS / aarch64-apple-darwin，rustc 1.98.1、cargo 1.98.1。这是本机检查记录，不替代或修改 Rust 1.99.1 候选基线；工具链固定属于 INFRA-002。
- 通过：`cargo fmt --all -- --check`、`cargo check --workspace`、`cargo test --workspace`、`cargo check --workspace --locked`、`cargo test --workspace --locked`、`cargo clippy --workspace --all-targets --locked -- -D warnings`。
- `cargo metadata --locked --format-version 1` 返回 25 个本地包；DFS 检查解析图无环，Domain 无依赖，UI 无直接 git-cli 依赖。
- 测试目标及文档测试成功，实际用例为 0。空骨架无业务行为或失败分支，未添加占位断言，不能视为业务验证。
- 测试链接时 Xcode 输出缓存路径与 FSEvents 环境警告，命令成功退出。
- 未验证 Rust 1.99.1、Windows/Linux、真实 UI、Git 功能或第三方依赖兼容性；本次没有公共业务 API。

## INFRA-002 当前平台验证（2026-11-06）

- Rust 1.99.1、Edition 2024、Resolver 3 和 GPUI-Kit 0.9.0 已在 macOS Apple Silicon 安装、解析并通过 Workspace 检查。
- 根清单统一核心依赖，Cargo.lock 锁定 819 个包；确切版本和命令见 [DEPENDENCY-VERIFICATION-001](../reviews/DEPENDENCY-VERIFICATION-001.md)。
- GPUI-Kit 实际解析到 gpui-pre 0.3.11、gpui-base/component/assets 0.9.0；编译探针核对了应用、Dock、Editor、虚拟列表和组件入口。
- git2 0.20 默认 features 因本机缺少 OpenSSL/pkg-config 失败；按本地只读适配器边界关闭默认远程 features 后，git2 0.20.4 编译通过。
- INFRA-002 完成时尚未验证 Windows/Linux；后续 INFRA-003 已补充三平台 CI 编译验证。真实窗口、MSRV、Git 能力矩阵和 libgit2 远程 features 仍未验证，不能标记 M0 完成。

## INFRA-003 三平台 CI（2026-11-06）

- 新增 GitHub Actions Workflow：独立格式检查以及 Ubuntu 24.04、Windows Server 2022、macOS 15 验证矩阵。
- 固定 Rust 1.99.1；metadata、check、Clippy 和 test 均使用 `--locked`，并检查 Cargo.lock 未变化。
- CI 记录 Runner、Rust、Git、CMake 及平台原生工具链；测试失败日志作为 Artifact 保留 7 天。
- 首次 GitHub Actions 运行中，Ubuntu 24.04、macOS 15 和格式检查通过；Windows 因 `windows-2025` 已迁移至 VS 2026，在原生工具链验证阶段失败。
- Workflow 按 VS 2022 基线改用 `windows-2022`；Rust CI #3 已完成修正后的三平台复验，Format 与三个验证 Job 全部通过。
- 官方 checkout 与 artifact Action 已升级到 Node.js 24 版本，消除首次运行的 Node.js 20 弃用警告。
- INFRA-003 已完成；成功证据见 [Rust CI #3](https://github.com/dengxiujun/IdeaGit/actions/runs/42235774214) 和 [CI-VERIFICATION-001](../reviews/CI-VERIFICATION-001.md)。
- GPUI 真实窗口与 Git Fixture 未执行，分别留给 UI-001 和 TEST-001。

## UI-001 GPUI-Kit 启动 Spike（2026-11-06）

- `idea-shell` 使用锁定的 GPUI-Kit 0.9.0 创建 900×600 最小原生窗口；desktop composition root 负责初始化、窗口生命周期和错误退出。
- 新增 `IDEAGIT_UI_SMOKE_DURATION_MS` 自动验证入口，在指定时间后显式绘制、关闭窗口并退出；无该环境变量时应用保持正常运行。
- 当前 macOS 15.8 / Apple Silicon 实际启动通过，窗口成功创建、绘制并正常关闭。
- [Rust CI #4](https://github.com/dengxiujun/IdeaGit/actions/runs/42251903525) 已在 Windows Server 2022、macOS 15 和 Ubuntu 24.04/Xvfb 完成窗口创建、绘制和关闭，Format 与三个验证 Job 全部通过。
- 通过 locked check、Clippy 和 Workspace tests；新增 2 个烟雾时限解析测试。
- UI-001 已完成；Windows 10/11、Linux Wayland/真实桌面以及输入、焦点、IME、DPI 仍属于后续平台与 UI 验证范围。
- 未实现 Design Tokens、Dock、Workbench、Git 功能、输入/焦点/IME 或视觉回归；详细结果见 [UI-VERIFICATION-001](../reviews/UI-VERIFICATION-001.md)。

## UI-002 IDEA Design Tokens（2026-11-07）

- `idea-theme` 新增暗/亮色语义色板、平台字体回退、Typography、逻辑像素 Metrics、完整 Theme Bundle 和可叠加交互状态。
- 颜色与逻辑像素使用强类型并拒绝非法输入；交互状态允许 Selected 与 Focused 等状态同时存在。
- 保持纯 Rust、零新增依赖；Cargo.toml 与 Cargo.lock 未变化，GPUI 映射和运行时切换留给后续 Primitive/Workbench 任务。
- 当前 macOS 通过 fmt、locked check、Clippy 和 Workspace tests；`idea-theme` 新增 7 个测试。
- [Rust CI #5](https://github.com/dengxiujun/IdeaGit/actions/runs/42256320592) 已在 Windows Server 2022、macOS 15 和 Ubuntu 24.04 通过 locked check、Clippy、Workspace tests 和窗口烟雾测试。
- UI-002 已完成；字体安装、中文回退、GPUI 映射、实际控件状态和 Golden 视觉测试仍属于后续 UI 验证范围。
- 工程初值不是 IDEA 2026.2 精确测量；实际字体、DPI、控件状态和 Golden 视觉测试仍未验证。详细记录见 [UI-TOKENS-VERIFICATION-001](../reviews/UI-TOKENS-VERIFICATION-001.md)。

## TEST-001 Git Fixture Harness（2026-11-07）

- 新增独立的 `tests/fixtures` workspace crate `git-fixture`，不被任何生产 crate 依赖；以 `TempDir` 创建真实 Git worktree，并在删除夹具时清理所有测试资料。
- 每个 `TestRepository` 为 Git 进程显式设置临时 HOME、USERPROFILE、GIT_CONFIG_GLOBAL、GIT_CONFIG_NOSYSTEM 和 GIT_TERMINAL_PROMPT；临时全局配置固定测试身份、空 Credential Helper、禁用 GPG 签名及专用 Hooks 目录。
- 当前 macOS 15.8 / Apple Silicon、Rust 1.99.1、Apple Git 2.39.5 已验证：Unborn HEAD、特殊路径、本地 bare remote 的 Push/Pull 往返、真实三方冲突、linked worktree 与 pre-commit Hook 拒绝，共 5 个真实 Git 测试。Windows 首次 CI 证实 Win32 API 不支持换行文件名，改用 Windows 合法的特殊路径后已由 Rust CI #8 复验通过。
- 当前平台已通过 fmt、locked check、Clippy（`-D warnings`）和 Workspace tests；详细结果及尚未验证范围见 [TEST-FIXTURE-VERIFICATION-001](../reviews/TEST-FIXTURE-VERIFICATION-001.md)。
- [Rust CI #8](https://github.com/dengxiujun/IdeaGit/actions/runs/42266728887) 已在 Windows Server 2022、macOS 15 和 Ubuntu 24.04 通过 Format、locked check、Clippy、Workspace tests 和既有窗口烟雾测试；TEST-001 已完成三平台验收。
- 真实 GPG/SSH 签名代理、凭据程序、网络远端、非 UTF-8 文件名及后续写操作故障注入仍属后续任务，不能据此开放标准写操作或高级 Index 功能。

## CORE-001 领域基础类型（2026-11-07）

- `git-domain` 新增强类型 `RepositoryId`、`WorktreeId`、`OperationId`、`SessionTarget` 和 `SnapshotRevision`，避免不同身份在 API 中混用；bare session 以 `worktree: None` 明确表达。
- `ObjectId` 显式保留 SHA-1/SHA-256 算法与完整摘要，拒绝缩写、非法长度、算法长度不匹配和非法十六进制；序列化使用规范小写全长形式。
- `RepoPath` 私有保存 Git 原始字节，构造和反序列化均拒绝空路径、NUL、绝对路径、空组件、`.` 与 `..`；特殊字符和非 UTF-8 字节无损。`LocalPath` 独立包装 OS 原生 `PathBuf`，两者不提供隐式转换。
- `GitErrorKind` 与 Core §18.6 完全对齐；`GitError` 保留脱敏消息/诊断、Session/Operation 上下文和结构化恢复动作，不用单一 retry 布尔值表达安全性。
- 当前 macOS / Rust 1.99.1 已通过 fmt、locked check、Clippy 与 Workspace tests；`git-domain` 新增 12 个正常/失败路径测试。
- [Rust CI #9](https://github.com/dengxiujun/IdeaGit/actions/runs/42275839590) 已在 Windows Server 2022、macOS 15 和 Ubuntu 24.04 通过全部门禁；CORE-001 已完成，详见 [CORE-001-VERIFICATION-001](../reviews/CORE-001-VERIFICATION-001.md)。

## CORE-003 ProcessRunner / GitCliExecutor（2026-11-07）

- `git-ports` 统一 `ExecutionContext`、`ProcessSpec`、环境敏感标记、stdin、原始输出、有界无内容进度和 `ProcessRunner`；参数保持 `OsString`，不经 Shell。
- `git-cli` 使用 Tokio 并发消费 stdout/stderr，分别限制输出，支持 deadline/timeout/cancellation，并在异常路径终止和回收完整进程树。
- Unix 使用 process group，Windows 使用 Job Object；精确新增 `process-wrap 10.0.1` 的最小 features，Cargo.lock 未改变既有依赖版本。
- `GitCliExecutor` 仅接受 sealed 强类型命令；本轮以真实 `git --version` 验证入口，不开放任意 Git 字符串或仓库写操作。
- 当前 macOS / Rust 1.99.1 已通过专项 fmt、locked check、Clippy 和 14 个测试；覆盖非 UTF-8、双管道大输出、非零退出、启动失败、超限、超时、取消、流式 stdin、提前退出和嵌套进程清理。
- Workspace 全量检查已通过；[Rust CI #10](https://github.com/dengxiujun/IdeaGit/actions/runs/42279427776) 已在 Windows Server 2022、macOS 15 和 Ubuntu 24.04 通过全部门禁及嵌套进程清理测试，CORE-003 已完成。详细记录见 [CORE-003-VERIFICATION-001](../reviews/CORE-003-VERIFICATION-001.md)。

## CORE-002 Repository Discovery（2026-11-07）

- `git-ports` 新增唯一的 `RepositoryLocation` 与 `RepositoryReader` 契约；bare 用 `None` 表达缺少 worktree，linked worktree 以共同的规范化 common_dir 表达共享协调域。
- `GitCliRepositoryReader` 只使用 Git 报告的 git_dir/common_dir/worktree root，再经文件系统 canonicalize；不拼接 `.git`，不经 Shell，并允许注入平台选择的 Git executable 与隔离环境。
- TEST-001 fixture 新增隔离环境导出和本地 submodule helper；生产 reader 集成测试不会读取用户全局 Git 配置。
- 当前 macOS / Rust 1.99.1 已通过专项 fmt、locked check、Clippy 和 21 个测试；真实 Git 覆盖 ordinary nested path、linked worktree、bare remote、submodule 及失败路径，Unix 路径解析保留内部换行。
- Workspace 全量检查已通过；[Rust CI #11](https://github.com/dengxiujun/IdeaGit/actions/runs/42313168974) 已在 Windows Server 2022、macOS 15 和 Ubuntu 24.04 通过全部门禁及真实仓库发现测试，CORE-002 已完成。详细记录见 [CORE-002-VERIFICATION-001](../reviews/CORE-002-VERIFICATION-001.md)。

## CORE-004 Porcelain v2 Status Parser（2026-11-07）

- `git-domain` 新增原始 ref、branch headers、XY、submodule、mode/OID、rename/copy score、conflict stages 和 `StatusData`；全零 mode/OID 表达为缺失。
- `git-ports` 新增 `StatusParser`、结构化 `StatusParseError`、`StatusOptions` 与异步 `StatusReader`，没有复制第二套 DTO。
- `git-cli` 严格按 NUL token 解析 porcelain v2；type-2 原路径单独取下一 token，未知 header 可扩展，截断或非法记录不静默接受。
- 生产 reader 使用 `--no-optional-locks`、明确 untracked/ignored/rename 选项；bare status 明确拒绝，不误报干净工作区。
- 当前 macOS / Rust 1.99.1 已通过专项 fmt、locked check、Clippy 和 43 个测试；合成输入覆盖所有格式/失败分支，隔离真实 Git 覆盖 staged+unstaged、rename、ignored/untracked、`UU` conflict 和 bare 拒绝。
- Workspace 全量检查已通过；[Rust CI #12](https://github.com/dengxiujun/IdeaGit/actions/runs/42315434816) 已在 Windows Server 2022、macOS 15 和 Ubuntu 24.04 通过全部门禁与真实 Git status 测试，CORE-004 已完成。详细记录见 [CORE-004-VERIFICATION-001](../reviews/CORE-004-VERIFICATION-001.md)。

## CORE-005 HEAD / Branch Reader（2026-11-07）

- `git-domain` 按 Core §18 新增 `Unborn`、`Attached`、`Detached` 三态 `HeadState`，并以独立 `BranchTracking` 保留 upstream 及 ahead/behind，不改变已批准契约。
- `git-ports` 新增异步 `HeadReader` 与 `BranchReader`；`git-cli` 通过 `symbolic-ref`、`rev-parse`、`for-each-ref` 和 `rev-list --left-right --count` 读取，不经 Shell，不执行 Git 写操作。
- Ref/OID/计数均严格校验；bare repository 使用真实 `git_dir`，未配置 upstream 或 detached HEAD 明确返回 `None`。
- Fixture 显式固定隔离的 `main` 默认分支，并新增 detach、peer-only push 与 fetch 辅助；真实测试覆盖 Unborn/Attached/Detached、bare、upstream 0/0 和分叉 1/1。
- 当前 macOS / Rust 1.99.1 的专项格式、53 个相关测试及包含 62 个测试的 Workspace 全量门禁已通过。
- [Rust CI #14](https://github.com/dengxiujun/IdeaGit/actions/runs/42319097515) attempt 2 已在 Windows Server 2022、macOS 15 和 Ubuntu 24.04 通过 Format、locked check、Clippy、Workspace tests 与窗口烟雾测试。首次 Windows bare HEAD 的 verbatim `GIT_DIR` 兼容问题已修复，CORE-005 已完成。详细记录见 [CORE-005-VERIFICATION-001](../reviews/CORE-005-VERIFICATION-001.md)。

## CORE-006 Snapshot Builder（2026-11-07）

- `git-domain` 新增批准契约中的 `RepositorySnapshot`、统一 `FileChange`、冲突元数据、worktree 可用性和一致性/陈旧标记；bare 空集合不会被解释为干净工作区。
- 纯 `SnapshotBuilder` 为整个结果分配同一个下一代 revision，并核对前后 HEAD guard、porcelain branch header 与独立 tracking；不一致生成带原因的 stale Snapshot，不伪装为原子 Git 状态。
- Status 投影保留 raw path、original path、双状态、OID/mode、Submodule、Rename/Copy score 与全部 conflict stages；Untracked 独立，Ignored 不进入批准的 Snapshot 合约。
- 当前 macOS / Rust 1.99.1 已通过专项格式、53 个相关单元测试、2 个真实 Git Snapshot 集成测试，以及包含 71 个测试的 Workspace 全量门禁；三平台 CI 尚待执行。详细记录见 [CORE-006-VERIFICATION-001](../reviews/CORE-006-VERIFICATION-001.md)。
