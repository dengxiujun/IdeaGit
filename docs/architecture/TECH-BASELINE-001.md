# IdeaGit — 核心技术与依赖版本基线

> 2026-11-06：执行已批准 [ADR D6](adr/ADR-ARCH-REMEDIATION-001.md)。保留候选版本，未安装/编译验证；日常工具链不等于已测 MSRV。实际依赖变更仍按 ADR 流程处理。

> 文档编号：TECH-BASELINE-001  
> 版本：v1.0  
> 日期：2026-11-06  
> 项目：IntelliJ IDEA 2026.x Git 体验复刻，独立 Rust + GPUI-Kit 桌面客户端  
> 平台：Windows / macOS / Linux

## 1. 版本基线与适用范围

本文件统一此前四份方案（总体架构、Git Core LLD、GPUI-Kit UI LLD、Commit/Changelist/Diff LLD，以及开发任务拆解）中涉及的工具链与主要依赖。它是**建议采用的开发基线**，而不是宣称已经对所有版本组合进行三平台构建与回归验证。

| 组件 | 建议版本或范围 | 用途和说明 |
| --- | --- | --- |
| Rust Toolchain | **1.99.1** | 建议固定的编译器版本；需通过实际安装与三平台构建确认 |
| Rust Edition | **2024** | 全部 Workspace Crate 统一采用 |
| Cargo Resolver | **3** | 与 Rust 2024 工作区匹配 |
| GPUI-Kit | **=0.9.0** | 桌面 UI 框架入口，建议精确固定 |
| GPUI 后端 | **gpui-pre 0.3.11** | 按此前选型记录的 GPUI-Kit 默认依赖；最终以 Cargo.lock 为准 |
| gpui-base | **0.9.0** | Dock、布局等能力；避免与 GPUI-Kit 使用不匹配的版本 |
| gpui-component | **0.9.0** | 通用 UI 组件；遵循 GPUI-Kit 的依赖版本 |
| gpui-kit-assets | **0.9.0** | 默认图标与资源；若项目使用则与 GPUI-Kit 对齐 |
| gpui-fast | **0.1.13（可选）** | 不作为初期默认后端，须单独验证 |
| Git CLI | **系统 Git** | Git 写入与语义兼容的权威执行引擎 |
| git2 | **0.20.x** | libgit2 的可选读取加速接口 |

### 版本说明

- 表中 `=0.9.0` 表示 Cargo 中使用精确版本要求。
- `1.x`、`0.20.x` 等是兼容版本范围，并非完整的 Patch 级锁定。
- `Cargo.lock` 才是每次实际构建中完整的传递依赖版本清单。
- 不建议自行拼接 `gpui-kit`、`gpui-pre`、`gpui-base`、`gpui-component` 的不同版本；应先验证对应锁文件中的依赖关系。
- 与 Git 操作相关的正确性仍以目标系统安装的 Git CLI 和真实仓库集成测试为准。

## 2. Rust 核心依赖

| Crate | 版本要求 | 主要用途 |
| --- | --- | --- |
| `tokio` | `1` | 异步运行时、进程、通道与任务 |
| `tokio-util` | `0.7` | `CancellationToken` 等工具 |
| `async-trait` | `0.1` | 对象安全异步 Trait 的适配 |
| `thiserror` | `2` | 结构化错误类型 |
| `serde` | `1` | 序列化/反序列化 |
| `serde_json` | `1` | 配置、事件与状态格式 |
| `uuid` | `1` | Repository、Worktree、Operation 标识 |
| `notify` | `8` | 文件系统监控 |
| `tracing` | `0.1` | 结构化日志与追踪 |
| `tracing-subscriber` | `0.3` | 日志订阅和过滤 |
| `git2` | `0.20` | 可选 libgit2 读取接口 |
| `tempfile` | `3` | 真实临时 Git 仓库测试 |
| `proptest` | `1` | 属性测试与解析器边界验证 |

正式编码时还可能增加 `secrecy`、`zeroize`、`rusqlite`、`lru` 等依赖；这些**未纳入本次已选定版本清单**，应通过 ADR 和三平台验证后再补充。

## 3. 推荐 `rust-toolchain.toml`

```toml
[toolchain]
channel = "1.99.1"
profile = "default"
components = ["rustfmt", "clippy"]
```

所有工程师本机与 CI 优先使用该固定工具链，避免构建差异。若采用更低 MSRV，应将 `workspace.package.rust-version` 与测试矩阵分开维护，不要误将“日常编译器版本”自动等同于“实际已验证的最低支持版本”。

## 4. 推荐 Workspace `Cargo.toml`

```toml
[workspace]
resolver = "3"
members = [
    "apps/desktop",
    "crates/core/*",
    "crates/adapters/*",
    "crates/ui/*",
    "crates/platform/*",
]

[workspace.package]
edition = "2024"
# rust-version 在独立 MSRV 验证后填写；不能把日常工具链自动当作已验证下限。
version = "0.1.0"
publish = false

[workspace.dependencies]
# GPUI 及上层 UI 框架
gpui-kit = "=0.9.0"

# 异步与任务
tokio = { version = "1", features = [
    "rt-multi-thread",
    "macros",
    "process",
    "sync",
    "time",
    "io-util",
    "fs",
] }
tokio-util = "0.7"
async-trait = "0.1"

# 领域模型与序列化
thiserror = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v4", "serde"] }

# Git 可选读取适配器
git2 = "0.20"

# 基础设施
notify = "8"
tracing = "0.1"
tracing-subscriber = "0.3"

# 测试
tempfile = "3"
proptest = "1"
```

> 注意：以上是**设计基线示例**，不是声称可直接通过三平台构建的已验证锁定清单；具体特性组合、平台原生依赖和传递依赖需要在真实 Workspace 内验证并锁定。

## 5. GPUI 依赖策略

1. 应用层以 `gpui-kit = "=0.9.0"` 作为统一 UI 入口。
2. 优先使用 GPUI-Kit 暴露或匹配的 GPUI API，不手动引入另一套不兼容 GPUI 版本。
3. `gpui-base`、`gpui-component`、`gpui-kit-assets` 若显式依赖，应与锁定的 GPUI-Kit 版本保持一致。
4. 版本升级必须验证 DockArea、虚拟列表、Code Editor、Popup、焦点、输入法和三平台窗口行为。
5. UI Golden Screenshot 与交互回归应关联具体的依赖锁文件和操作系统环境。

### `gpui-fast` 的可选评估

不建议在首个生产迭代中默认开启。若锁定版本实际提供该 Feature，可在专门性能分支尝试：

```toml
gpui-kit = { version = "=0.9.0", features = ["gpui-fast"] }
```

评估重点：Windows / macOS / Linux 构建、Dock 拖拽、Diff Viewer、Commit Graph、虚拟列表、输入法、GPU 渲染及稳定性。实际 Feature 名与依赖能力必须以当前版本清单为准。

## 6. Git CLI 与 libgit2 版本策略

- 以用户系统的 **Git CLI** 执行 Stage、Commit、Checkout、Fetch、Push、Merge、Rebase 等写入或网络操作。
- `git2 = "0.20"` 仅作为可选读取优化方案，不绕过统一 `OperationCoordinator` 修改 Index 或 Refs。
- 应在启动时检测系统 Git 版本与可用功能，提供明确的最低支持版本策略；**本文件尚未锁定 Git CLI 的最低版本号**。
- 针对 SHA-256 Repository、Worktree、Submodule、Git LFS、Signed Commit、Sparse Checkout 等能力建立兼容矩阵。
- 每个写操作完成后重新读取仓库状态，不能将成功的子进程退出码直接等同于整个业务状态已一致。

## 7. 版本锁定与升级规则

1. 提交 `Cargo.lock` 到仓库。
2. CI 使用 `--locked` 验证依赖没有漂移。
3. 固定 Rust 工具链，统一 Rust 2024 Edition。
4. 优先固定 GPUI-Kit 的精确版本，避免手动混用 GPUI 后端。
5. 对依赖升级开启独立 PR，记录变更原因与回归结果。
6. 仅当三平台编译、基础 UI 自动化和 Git Core 集成测试都通过时才接受主版本或关键次版本升级。
7. Git Core / Commit Transaction 依赖升级须执行数据安全回归，尤其是 Index、Partial Commit、Recovery。
8. 保留上一版可重复构建环境，出现严重回归时可回滚依赖变更。

推荐验证命令：

```bash
rustc --version
cargo --version
cargo metadata --locked --format-version 1 > /dev/null
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

## 8. 三平台验证门槛

| 验证项 | Windows | macOS | Linux |
| --- | :---: | :---: | :---: |
| Workspace `cargo check` | 必须 | 必须 | 必须 |
| UI 空窗口启动 | 必须 | 必须 | 必须 |
| Dock / Popup / 输入与焦点 | 必须 | 必须 | 必须 |
| Git CLI 集成测试 | 必须 | 必须 | 必须 |
| 文件监控与仓库刷新 | 必须 | 必须 | 必须 |
| 真实 Git Commit / Branch / Diff | 必须 | 必须 | 必须 |
| Credential / SSH / HTTPS | 必须 | 必须 | 必须 |
| 关键 UI 回归 | 必须 | 必须 | 必须 |
| DPI 与平台特性 | 必须 | 必须 | 必须 |

Linux 还需在明确的发行版基线上安装所需开发库并验证 Wayland/X11 场景；macOS 与 Windows 应验证原生路径、签名及安装包相关运行行为。

## 9. 当前验证状态与下一步

| 内容 | 状态 |
| --- | --- |
| Rust 2024、Resolver 3 作为设计选型 | 已明确 |
| Rust 1.99.1 作为建议工具链 | 已在 macOS Apple Silicon 安装并完成 Workspace 编译验证 |
| GPUI-Kit 0.9.0 作为建议 UI 版本 | 已在当前 macOS 平台解析并完成编译/API 探针验证 |
| gpui-pre 0.3.11 的实际解析版本 | 已由 `Cargo.lock` 核对 |
| `tokio` / `git2` / `notify` 等依赖 | 已锁定实际 Patch；见 DEPENDENCY-VERIFICATION-001 |
| Windows / Linux 构建及三平台真实窗口 | **未验证** |
| IDEA 2026.x 与 UI API 的行为对齐 | 待真实样本与回归测试 |

**正式冻结基线的步骤：** 建立可编译 Workspace → 生成并提交 Cargo.lock → 三平台 CI 构建 → 启动 GPUI 窗口和 Dock → 运行真实 Git 仓库测试 → 记录测试环境与最终版本清单 → 将本文状态更新为“已验证”。

---

## 10. 推荐结论

第一版建议使用 **Rust 1.99.1 + Edition 2024 + Cargo Resolver 3 + GPUI-Kit 0.9.0**，后端以 **系统 Git CLI** 为权威执行路径，`git2 0.20.x` 只作为可选只读加速；异步与文件监控基于 Tokio 1.x、notify 8.x。

这一组合是**待三平台 CI 验证的开发基线**。最终生产发布时，应以真实 `Cargo.lock`、Git 兼容矩阵、GPUI UI 回归和 Windows/macOS/Linux 完整验收记录为准。

## 11. ADR 修订后的验证清单

- 日常开发与 CI 固定 Rust 1.99.1；禁止使用滚动 stable 替代。候选不兼容需另提 ADR，不自动升降核心版本。
- MSRV 未验证；Edition/Resolver 选择不证明所有传递依赖的最低编译器要求。
- Git 最低版本/能力矩阵尚待 INFRA-002 提案：逐项验证 restore、pathspec-from-file、SHA-256、worktree、过滤器、签名等；版本/能力不足时拒绝对应功能，不拼凑不安全降级。
- Tokio/tokio-util/Serde/UUID 等保留已有范围；所有实际 patch、feature、MSRV、原生依赖记录在后续依赖验证报告与 Cargo.lock。arc-swap/parking_lot/sha2/secrecy/SQLite 等未冻结需求必须明确归属及审批，不因示例引用就隐式引入。
- GPUI 使用 Kit 匹配的重导出；Dock/Panel/renderer、Editor、VirtualList、test-support 的存在不等于应用接入通过。
- Windows 记录 MSVC/SDK/CMake；macOS 记录 OS/Xcode；Linux 记录发行版、原生库、X11/Wayland/Vulkan 图形环境。cargo check 与原生窗口/IME/焦点/截图分别记录结果。
- INFRA-002 已在 macOS Apple Silicon 使用 Rust 1.99.1 完成依赖解析、API 探针和 Workspace 检查；详见 [DEPENDENCY-VERIFICATION-001](../reviews/DEPENDENCY-VERIFICATION-001.md)。Windows、Linux、真实窗口和 MSRV 仍未验证。
