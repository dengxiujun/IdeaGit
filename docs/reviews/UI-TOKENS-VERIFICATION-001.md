# UI-TOKENS-VERIFICATION-001：UI-002 Design Tokens 验证记录

- 日期：2026-11-07
- 任务：UI-002
- 状态：已完成；当前 macOS 检查及 Rust CI #5 三平台验证全部通过

## 1. 范围

本任务只在 `idea-theme` 建立 IDEA-inspired Design Tokens，包括暗色/亮色语义颜色、Typography、逻辑像素 Metrics 和可叠加交互状态。没有实现运行时主题切换、GPUI 类型转换、控件、Dock、Workbench、图标或 Git 功能。

这些值是 LLD 批准的工程初值及其配套色板，不是对 IntelliJ IDEA 2026.2 的精确像素或颜色测量。正式视觉基线仍需截图采样、授权资产审查和多 DPI Golden 测试。

## 2. 公共 Token

### 颜色

`ColorToken` 使用经过范围验证的 `0xRRGGBB`，防止高位或 Alpha 编码被无意混入。`IdeaColors::light()` 与 `IdeaColors::dark()` 提供：

- Window、Panel、Editor 背景。
- Border、Separator、Primary/Secondary/Disabled Text。
- Selection、Hover、Focus。
- Git Added、Modified、Deleted、Conflicted。
- Busy 与 Error 语义色。

### Typography

`IdeaTypography::platform_default()` 提供 12/13/16 逻辑像素字号、18/20 行高和 Regular/Medium/Semibold 字重。字体按目标平台编译选择有序回退：

- Windows：Segoe UI、Microsoft YaHei UI。
- macOS：系统 UI 字体、PingFang SC。
- Linux：Inter、Noto Sans、Noto Sans CJK SC。
- 等宽字体均优先 JetBrains Mono，并保留平台字体与通用 `monospace` 回退。

字体存在性和实际排版不由 Token 声明保证，仍需后续真实系统验证。

### Metrics

`IdeaMetrics::initial()` 使用逻辑像素而非物理像素：Toolbar 36、Status Bar 26、Tree Row 24、Compact Row 22、Tab 30、Panel Header 32、Icon 16、Panel Padding 8、Splitter 4。

`LogicalPixels` 拒绝零、负数、NaN 和无穷值，防止无效尺寸进入后续 UI Primitive。

### 交互状态

`InteractionState` 允许 Hovered、Selected、Focused、Disabled、Busy、Error 组合存在，不把 Selected 与 Focused 建模为互斥枚举。

## 3. 设计选择

- `idea-theme` 保持纯 Rust 和零第三方依赖，避免 Design Tokens 直接绑定某个 GPUI 后端。
- `IdeaTheme` 将 ThemeMode、颜色、Typography 和 Metrics 组合为不可变值；GPUI 转换由后续 `idea-ui-primitives` 负责。
- 没有修改核心版本、Crate 边界或既有依赖方向，不需要新 ADR。
- Cargo.toml 与 Cargo.lock 均未变化。

## 4. 实际验证

当前环境为 macOS 15.8 / Apple Silicon，Rust 1.99.1。实际结果：

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo check --workspace --all-targets --locked` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 |
| `cargo test --workspace --locked` | 通过；`idea-theme` 新增 7 个测试 |

新增测试覆盖暗/亮色差异、非法 RGB、Metrics 初值、非法逻辑像素、平台字体安全回退、交互状态叠加和完整主题选择。

## 5. 三平台验证

[Rust CI #5](https://github.com/dengxiujun/IdeaGit/actions/runs/42256320592) 对提交 `ee30de0` 执行固定 Rust 1.99.1 的 locked metadata、check、Clippy、Workspace tests、Cargo.lock 检查和 UI-001 窗口烟雾测试，结果全部成功：

| Job | 结果 | 证据 |
| --- | --- | --- |
| Format | 通过 | [Job](https://github.com/dengxiujun/IdeaGit/actions/runs/42256320592/job/128024352473) |
| Windows Server 2022 | 通过 | [Job](https://github.com/dengxiujun/IdeaGit/actions/runs/42256320592/job/128024352756) |
| macOS 15 | 通过 | [Job](https://github.com/dengxiujun/IdeaGit/actions/runs/42256320592/job/128024352940) |
| Ubuntu 24.04 | 通过 | [Job](https://github.com/dengxiujun/IdeaGit/actions/runs/42256320592/job/128024353103) |

Windows、macOS 和 Linux 的条件编译字体列表及 Token 测试均已通过，UI-002 的工程验收条件满足。

## 6. 尚未验证

- 字体是否安装、中文回退、字形宽度、截断和排版结果。
- GPUI 映射、应用内主题切换、系统主题跟随和持久化。
- Hover/Selected/Focused/Disabled/Busy/Error 的真实控件渲染。
- IDEA 2026.2 精确颜色、字体、间距、Windows 缩放、macOS Retina、Linux X11/Wayland 和截图 Golden。
