# CI-VERIFICATION-001：INFRA-003 三平台 CI 验证记录

- 日期：2026-11-06
- 任务：INFRA-003
- 状态：Workflow 已建立，本机检查通过；GitHub 托管 Runner 首次运行待推送后验证

## 1. 范围

本任务建立 Windows、macOS、Linux 三平台最小工程 CI，只验证工具链、锁文件、Workspace 编译、Clippy 和现有测试。它不启动 GPUI 窗口，不实现 Git Fixture，不验证 GPU、输入、IME、焦点或真实 Git 行为。

Workflow 位于 `.github/workflows/rust.yml`，包含独立格式检查和三平台验证矩阵。

## 2. 固定环境

| 项目 | 配置 |
| --- | --- |
| Rust | 1.99.1，minimal profile |
| Edition / Resolver | 2024 / 3 |
| Linux | `ubuntu-24.04` |
| Windows | `windows-2025`，要求 `x86_64-pc-windows-msvc` |
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

矩阵使用 `fail-fast: false`，单个平台失败不会取消其他平台，便于收集完整兼容性结果。测试失败时上传对应系统的 Cargo 测试日志，保留 7 天；Workflow 权限仅为 `contents: read`，不读取或上传凭据。

## 4. 当前验证结果

| 验证项 | 状态 | 证据 / 限制 |
| --- | --- | --- |
| Workflow YAML 可解析 | 已通过 | 本机 YAML 解析检查 |
| Rust 格式检查 | 已通过 | 当前 macOS 本机执行 |
| Workspace check | 已通过 | 当前 macOS 本机执行，含 all-targets 与 locked |
| Clippy | 已通过 | 当前 macOS 本机执行，`-D warnings` |
| Workspace tests | 已通过 | 当前 macOS 本机执行；骨架实际测试用例为 0 |
| Ubuntu 24.04 Runner | 待执行 | Workflow 推送到 GitHub 后取得运行记录 |
| Windows Server 2025 Runner | 待执行 | Workflow 推送到 GitHub 后取得运行记录 |
| macOS 15 Runner | 待执行 | Workflow 推送到 GitHub 后取得运行记录 |

本地命令不能代替 GitHub 托管 Runner 结果。在三个矩阵 Job 实际为绿色并记录运行链接之前，INFRA-003 不标记完成，不声称三平台构建已经通过。

## 5. 完成证据

首次 GitHub Actions 运行后，应在本节补充：

| Runner | Actions 运行链接 | ImageVersion | 结果 |
| --- | --- | --- | --- |
| ubuntu-24.04 | 待补充 | 待补充 | 未执行 |
| windows-2025 | 待补充 | 待补充 | 未执行 |
| macos-15 | 待补充 | 待补充 | 未执行 |

只有三个平台均通过，Cargo.lock 无变化，失败日志策略实际可用，才能将 INFRA-003 标记为完成。

## 6. 明确未验证

- Windows 10/11、真实 macOS 桌面和 Linux Wayland/X11 会话中的 GPUI 窗口运行。
- GPU 渲染、DPI、输入、IME、焦点、Dock、Editor 和虚拟列表行为。
- 真实 Git Fixture、用户 Git 配置隔离和 Git 写操作。
- 安装包、签名、公证、升级和生产发布。
- MSRV；CI 验证的是固定日常工具链 Rust 1.99.1。

真实窗口属于 UI-001，Git Fixture 属于 TEST-001，均不在本任务实现范围内。
