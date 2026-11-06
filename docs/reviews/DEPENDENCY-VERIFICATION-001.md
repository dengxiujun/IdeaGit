# DEPENDENCY-VERIFICATION-001：Rust 与核心依赖兼容性验证

- 日期：2026-11-06
- 任务：INFRA-002
- 当前平台：macOS 15.8（24H23），Apple Silicon / `aarch64-apple-darwin`
- 状态：当前平台编译验证通过；Windows、Linux 和真实窗口运行未验证

## 1. 结论

候选组合 Rust 1.99.1、Edition 2024、Cargo Resolver 3、GPUI-Kit 0.9.0 在当前 macOS 平台可解析并通过 Workspace 编译、测试和 Clippy。没有升级核心依赖的大版本，也没有改变模块边界。

`rust-toolchain.toml` 固定 Rust 1.99.1、minimal profile、rustfmt 和 clippy。`Cargo.lock` 由 Cargo 1.99.1 生成，共锁定 819 个包。该结果只证明当前锁文件与当前平台可构建，不代表 MSRV 或三平台已验证。

## 2. 工具链与环境

| 项目 | 实际结果 |
| --- | --- |
| rustc | 1.99.1 (`58cf8f9c3 2026-10-13`) |
| cargo | 1.99.1 (`5f94df478 2026-08-27`) |
| rustfmt | 1.10.0-stable (`58cf8f9c35 2026-10-13`) |
| clippy | 0.1.99 (`58cf8f9c35 2026-10-13`) |
| Edition / Resolver | 2024 / 3 |
| Xcode | 16.4（16F6） |
| 系统 Git | 2.39.5（Apple Git-154） |

Rust 1.99.1 可从 rustup 官方渠道发现并安装，项目目录中的工具链文件会选择 `1.99.1-aarch64-apple-darwin`。没有设置 `workspace.package.rust-version`，因为本任务没有执行独立 MSRV 验证。

## 3. 直接依赖的锁定版本

| Workspace 要求 | Cargo.lock 实际版本 | 当前用途 |
| --- | --- | --- |
| `gpui-kit = "=0.9.0"` | 0.9.0 | Desktop 统一 UI 入口 |
| `tokio = "1"` | 1.54.1 | Desktop / Ports 异步基础 |
| `tokio-util = "0.7"` | 0.7.20 | Ports 取消能力；启用 `rt` |
| `async-trait = "0.1"` | 0.1.92 | Ports 异步 Trait |
| `thiserror = "2"` | 2.0.21 | Domain 结构化错误 |
| `serde = "1"` | 1.0.229 | Domain / Persistence 序列化 |
| `serde_json = "1"` | 1.0.154 | Persistence JSON |
| `uuid = "1"` | 1.29.0 | Domain 身份；启用 `v4`、`serde` |
| `notify = "8"` | 8.2.1 | Repository watcher |
| `tracing = "0.1"` | 0.1.44 | Desktop 追踪 |
| `tracing-subscriber = "0.3"` | 0.3.23 | Desktop 日志订阅 |
| `git2 = "0.20"` | 0.20.4 | 可选本地只读适配器 |
| `tempfile = "3"` | 3.27.0 | Desktop 测试依赖 |
| `proptest = "1"` | 1.12.0 | Desktop 测试依赖 |

锁文件中同时存在传递依赖 `thiserror 1.0.69`；项目直接依赖仍为 2.0.21。这是依赖树的版本并存，不是 Workspace 直接依赖漂移。

## 4. GPUI-Kit 依赖与 API 核验

crates.io 发布包的实际清单显示：

- `gpui-kit 0.9.0` 默认 features 为 `component` 和 `assets`。
- 默认后端精确依赖 `gpui-pre 0.3.11`、`gpui-pre-macros 0.3.11` 和桌面 `gpui-pre-platform 0.3.11`。
- 配套层解析为 `gpui-base 0.9.0`、`gpui-component 0.9.0`、`gpui-kit-assets 0.9.0`。
- `gpui-fast 0.1.13` 是可选 feature，本次没有启用。
- Desktop 只直接依赖 `gpui-kit`，GPUI API 通过 `gpui_kit::*`，Base、Component 和 Assets 分别通过 `gpui_kit::base`、`component`、`assets` 访问，不单独混装后端。

编译探针 `apps/desktop/examples/dependency_api_check.rs` 验证了以下实际路径：`application`、`init`、`WindowOptions`、`Render`、`base::dock::DockArea`、`base::input::EditorState`、`base::VirtualList`、`component::input::Editor` 和 `component::button::Button`。探针不创建窗口，不构成 UI-001 的窗口、渲染、输入、焦点或 Dock 运行验证。

依据：

- [GPUI-Kit 0.9.0 安装说明](https://gpui-kit.com/docs/installation/)
- [GPUI-Kit 0.9.0 发布包](https://crates.io/crates/gpui-kit/0.9.0)
- 本次下载的 crates.io 发布清单及 `Cargo.lock`

## 5. 冲突与最小修正

首次按 `git2 = "0.20"` 默认 features 编译时，`openssl-sys 0.9.118` 因当前环境没有 `pkg-config` 和可发现的 OpenSSL 开发目录而失败。默认 features 包含远程传输能力，但本项目的 `git-libgit2` 定位是可选本地只读加速，远程和写操作由系统 Git CLI 负责。

最小修正为保留 `git2 0.20`，配置 `default-features = false`。修正后解析为 `git2 0.20.4`、`libgit2-sys 0.18.8+1.9.7`，当前平台编译通过。未来若要让 libgit2 承担 HTTPS/SSH 读取，必须单独核对三平台 TLS/SSH 原生依赖并重新验证；本次不开放该能力。

未发现需要更换 Rust、GPUI-Kit 或其他核心依赖版本的阻断冲突，因此没有提出 ADR。

## 6. 实际验证

以下命令均使用项目固定的 Rust 1.99.1 和已提交锁文件：

| 命令 | 结果 |
| --- | --- |
| `cargo metadata --locked --format-version 1 --no-deps` | 通过 |
| `cargo check --workspace --all-targets --locked` | 通过，包括编译 API 探针 |
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 |
| `cargo test --workspace --locked` | 通过；当前骨架实际测试用例为 0 |
| `git diff --check` | 通过 |

首次下载和编译后的增量 `cargo check` 成功；未运行真实 GPUI 窗口，也未执行 Git 功能测试。

## 7. 尚未验证

- Windows 10/11 MSVC、Visual Studio 2022、Windows SDK、CMake 和窗口运行。
- Ubuntu 24.04 的 X11/Wayland、Vulkan、fontconfig、WebKitGTK 等原生依赖及窗口运行。
- macOS Intel、其他 macOS/Xcode 组合、签名和公证。
- GPUI Dock 拖拽、虚拟列表、Editor、IME、焦点、输入、渲染和关闭生命周期。
- `gpui-fast` 后端、`test-support` feature 和截图测试。
- Rust 1.99.1 是否为 MSRV；当前结果只验证固定日常工具链。
- 系统 Git 最低版本和 restore、pathspec-from-file、SHA-256、worktree、过滤器、签名能力矩阵。
- git2 的 HTTPS/SSH features；当前明确关闭。

这些事项分别由 INFRA-003、UI-001、TEST-001 及后续 Git 能力任务验证，不能由当前平台的 `cargo check` 推断为已通过。
