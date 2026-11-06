# CI-VERIFICATION-001：INFRA-003 三平台 CI 验证记录

- 日期：2026-11-06
- 任务：INFRA-003
- 状态：已完成；Rust CI #3 的格式检查及 Ubuntu、Windows、macOS 验证全部通过

## 1. 范围

本任务建立 Windows、macOS、Linux 三平台最小工程 CI，只验证工具链、锁文件、Workspace 编译、Clippy 和现有测试。它不启动 GPUI 窗口，不实现 Git Fixture，不验证 GPU、输入、IME、焦点或真实 Git 行为。

Workflow 位于 `.github/workflows/rust.yml`，包含独立格式检查和三平台验证矩阵。

## 2. 固定环境

| 项目 | 配置 |
| --- | --- |
| Rust | 1.99.1，minimal profile |
| Edition / Resolver | 2024 / 3 |
| Linux | `ubuntu-24.04` |
| Windows | `windows-2022`，要求 VS 2022 与 `x86_64-pc-windows-msvc` |
| macOS | `macos-15` |
| Cargo.lock | 所有解析、编译和测试命令使用 `--locked` |

CI 显式输出 Runner image、rustc、cargo、Git 和 CMake 版本。Windows 额外检查 Visual Studio C++ Build Tools、Windows SDK 和 MSVC Rust host；Linux 安装并记录 GPUI-Kit 所需的编译、字体、Wayland、X11、WebKitGTK、TLS 和 Vulkan 开发环境；macOS 输出 Xcode 版本。

没有配置 `gpui-fast`，也没有改变 INFRA-002 已锁定的依赖版本。

## 3. 执行命令

格式 Job 执行：

```text
cargo fmt --all -- --check
```

三个系统分别执行：

```text
cargo metadata --locked --format-version 1 --no-deps
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
git diff --exit-code -- Cargo.lock
```

矩阵使用 `fail-fast: false`，单个平台失败不会取消其他平台，便于收集完整兼容性结果。测试失败时上传对应系统的 Cargo 测试日志，保留 7 天；Workflow 权限仅为 `contents: read`，不读取或上传凭据。`actions/checkout@v6` 与 `actions/upload-artifact@v6` 使用 Node.js 24，避免首次运行中 v4 Action 的 Node.js 20 弃用警告。

## 4. 当前验证结果

| 验证项 | 状态 | 证据 / 限制 |
| --- | --- | --- |
| Workflow YAML 可解析 | 已通过 | 本机 YAML 解析检查 |
| Rust 格式检查 | 已通过 | 当前 macOS 本机执行 |
| Workspace check | 已通过 | 当前 macOS 本机执行，含 all-targets 与 locked |
| Clippy | 已通过 | 当前 macOS 本机执行，`-D warnings` |
| Workspace tests | 已通过 | 当前 macOS 本机执行；骨架实际测试用例为 0 |
| Ubuntu 24.04 Runner | 已通过 | Rust CI #3 完成 metadata、check、Clippy、test 与锁文件检查 |
| Windows Server 2022 Runner | 已通过 | Rust CI #3 完成原生工具链检查、metadata、check、Clippy、test 与锁文件检查 |
| macOS 15 Runner | 已通过 | Rust CI #3 完成 metadata、check、Clippy、test 与锁文件检查 |

GitHub 托管 Runner 的 Rust CI #3 已提供三平台实际构建证据，因此 INFRA-003 标记完成。该结果只证明当前最小工程在固定 Runner 镜像和 Rust 1.99.1 下通过本 Workflow 的构建与测试命令，不等同于真实图形窗口运行验证。

### 首次运行发现

首次运行：[Rust CI #1](https://github.com/dengxiujun/IdeaGit/actions/runs/42230805543)。

- Format、Ubuntu 24.04、macOS 15 成功。
- Windows 在 `Verify Windows native toolchain` 失败，尚未进入 Cargo 构建。
- 原因是 GitHub 已将 `windows-2025` 标签迁移到 Visual Studio 2026 镜像，而 Workflow 按 GPUI-Kit 当前基线要求筛选 VS 2022（`[17.0,18.0)`）。
- 最小修正是使用 GitHub 官方仍提供 VS 2022 的 `windows-2022`，不放宽工具链检查，也不改变 Rust 或项目依赖版本。

### 修正后运行结果

- Rust CI #2 中，Windows Server 2022、Ubuntu 24.04 和格式检查通过；macOS Job 在分配 Runner 前被取消，未执行项目命令，因此该次运行不能作为完整三平台通过证据。
- 手动触发的 [Rust CI #3](https://github.com/dengxiujun/IdeaGit/actions/runs/42235774214) 使用同一提交 `3839006`，四个 Job 均成功完成。
- 三个平台均完成 locked metadata、Workspace check、Clippy、测试和 Cargo.lock 未变化检查；Format Job 也成功完成。

## 5. 完成证据

| Runner | Actions 运行链接 | ImageVersion | 结果 |
| --- | --- | --- | --- |
| Format / ubuntu-24.04 | [Rust CI #3 Job](https://github.com/dengxiujun/IdeaGit/actions/runs/42235774214/job/127952861670) | 见 Job 日志的 `Set up job` | 通过 |
| ubuntu-24.04 | [Rust CI #3 Job](https://github.com/dengxiujun/IdeaGit/actions/runs/42235774214/job/127952861193) | 见 `Show runner environment` 日志 | 通过 |
| windows-2022 | [Rust CI #3 Job](https://github.com/dengxiujun/IdeaGit/actions/runs/42235774214/job/127952861766) | 见 `Show runner environment` 日志 | 通过 |
| macos-15 | [Rust CI #3 Job](https://github.com/dengxiujun/IdeaGit/actions/runs/42235774214/job/127952861685) | 见 `Show runner environment` 日志 | 通过 |

Rust CI #3 于 2026-11-06 完成，结论为 `success`。三个验证 Job 均通过且 Cargo.lock 无变化，INFRA-003 的三平台 CI 验收条件已满足。失败日志上传步骤由 `if: failure()` 控制；本次成功运行未产生失败 Artifact，这是预期行为。

## 6. 明确未验证

- Windows 10/11、真实 macOS 桌面和 Linux Wayland/X11 会话中的 GPUI 窗口运行。
- GPU 渲染、DPI、输入、IME、焦点、Dock、Editor 和虚拟列表行为。
- 真实 Git Fixture、用户 Git 配置隔离和 Git 写操作。
- 安装包、签名、公证、升级和生产发布。
- MSRV；CI 验证的是固定日常工具链 Rust 1.99.1。

真实窗口属于 UI-001，Git Fixture 属于 TEST-001，均不在本任务实现范围内。
