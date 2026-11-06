# IdeaGit 开发进度

更新日期：2026-11-07。

## 当前状态

INFRA-001/002 已建立 Workspace 并锁定依赖；INFRA-003/004 已完成；UI-001 已完成三平台窗口启动验收。UI-002 Design Tokens 已在当前 macOS 实现并通过 Workspace 检查，等待统一推送后的 Windows/Linux CI 复验。M0 整体仍未完成，剩余工程门禁为 TEST-001 Git Fixture 和 UI-002 三平台复验。

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

提交 UI-002 后执行 TEST-001 Git Fixture；统一推送后取得 UI-002 三平台 CI 结果并完成对应收尾。一次只实施一个已跟踪任务，核心版本或边界变更仍需 ADR。

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
- Windows/Linux 条件编译字体列表尚未取得 CI 结果，统一推送并通过三平台 CI 前不能将 UI-002 标记为最终完成。
- 工程初值不是 IDEA 2026.2 精确测量；实际字体、DPI、控件状态和 Golden 视觉测试仍未验证。详细记录见 [UI-TOKENS-VERIFICATION-001](../reviews/UI-TOKENS-VERIFICATION-001.md)。
