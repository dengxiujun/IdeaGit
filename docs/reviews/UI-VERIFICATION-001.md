# UI-VERIFICATION-001：GPUI-Kit 最小窗口验证记录

- 日期：2026-11-06
- 任务：UI-001
- 状态：当前 macOS 实际运行通过；Windows、Linux 原生窗口烟雾测试等待 CI 复验

## 1. 范围

本任务只建立 GPUI-Kit 0.9.0 最小原生窗口和可重复的启动烟雾测试。窗口显示 IdeaGit 名称及 Spike 标识，不包含 Workbench、Dock、Design Tokens、真实 Git、业务交互或布局持久化。

`apps/desktop` 负责 GPUI 应用生命周期和失败退出码，`idea-shell` 负责创建最小窗口及渲染根。UI 仍未直接依赖 `git-cli`，没有改变 Git Core 或 Application 边界。

## 2. 实现

- desktop composition root 调用 `gpui_kit::application()`、`gpui_kit::init()` 和 `idea_shell::open_main_window()`。
- 应用使用 `QuitMode::LastWindowClosed`，确保最后一个窗口关闭后在三平台一致退出。
- 窗口尺寸为 900×600 逻辑像素，最小尺寸为 640×420，使用原生标题栏并设置 `com.ideagit.desktop` 应用标识。
- `IDEAGIT_UI_SMOKE_DURATION_MS` 仅用于自动化运行验证：等待指定时间、显式绘制窗口、关闭窗口并退出。缺失时保持正常交互运行；零值或非数字会以失败退出。
- CI 在 macOS 和 Windows 直接运行桌面二进制；Linux 使用 Xvfb 提供 X11 显示环境。所有 Cargo 命令继续使用 `--locked`。

## 3. 当前平台实际运行

| 项目 | 实际环境 |
| --- | --- |
| 操作系统 | macOS 15.8（24H23） |
| 架构 | Apple Silicon / `aarch64-apple-darwin` |
| Xcode | 16.4（16F6） |
| Rust / Cargo | 1.99.1 / 1.99.1 |
| GPUI-Kit / GPUI | 0.9.0 / gpui-pre 0.3.11 |

执行命令：

```text
IDEAGIT_UI_SMOKE_DURATION_MS=15000 cargo run --locked -p desktop
```

实际结果：进程报告 `IdeaGit native window opened`，原生窗口创建成功；15 秒后完成显式绘制并报告 `IdeaGit native window rendered and closed after smoke duration`，进程以成功状态退出。

这证明当前 macOS 环境中的应用初始化、窗口创建、基础渲染和关闭生命周期可运行，不证明输入、焦点、IME、Dock、DPI 或业务 UI 已通过。

## 4. 自动检查

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo check --workspace --all-targets --locked` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 |
| `cargo test --workspace --locked` | 通过；新增 2 个烟雾时限解析测试 |

Cargo.lock 只新增 `idea-shell` 对既有 `gpui-kit 0.9.0` 的直接依赖记录，没有改变任何第三方依赖版本。

## 5. 三平台状态

| 平台 | 状态 | 限制 |
| --- | --- | --- |
| macOS 15 / Apple Silicon | 已通过 | 当前开发机实际创建、绘制并关闭原生窗口 |
| Windows Server 2022 | 待复验 | Workflow 已加入可见原生窗口烟雾命令，尚未取得 Runner 结果 |
| Ubuntu 24.04 / X11 | 待复验 | Workflow 已加入 Xvfb 原生窗口烟雾命令，尚未取得 Runner 结果；Wayland 未覆盖 |

只有推送后 Windows、macOS、Linux 三个 Runner 的窗口烟雾步骤全部成功，UI-001 才能标记完成。GitHub 托管 Runner 结果也不替代 Windows 10/11、真实 Linux 桌面、IME、DPI 和 GPU 兼容性验证。

## 6. 明确未实现

- UI-002 的 IDEA 暗/亮主题、Typography 和 Metrics。
- DockArea、Workbench、Repository Tabs、StatusBar、Overlay 和布局恢复。
- GitApplication 注入、仓库打开、Git 状态或任何 Git 写操作。
- 键盘、鼠标、焦点、输入法、辅助功能、截图 Golden 和多显示器验证。
- Windows/Linux 真实桌面设备运行；需在 CI 复验之后继续保留该限制。
