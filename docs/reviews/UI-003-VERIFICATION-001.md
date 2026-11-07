# UI-003-VERIFICATION-001：Workbench Shell / DockArea 验证记录

- 日期：2026-11-07
- 任务：UI-003
- 当前状态：当前 macOS 实现及 Workspace 全量门禁通过；本提交三平台 CI 待执行

## 1. 范围与权威契约

本任务实现 LLD-UI-001 §5、§16 UI-M1 与 IP-001 §4.6 的 Workbench Shell：GPUI-Kit DockArea、稳定 PanelRegistry、Titlebar/Repository Tab 外壳、StatusBar、OverlayHost、版本化布局保存和恢复。

UI-003 不直接调用 Git CLI，也不伪造 Repository、HEAD 或 Changes。真实 Repository Tabs/StatusBar 数据属于 UI-004，Read-only Changes Tree 属于 UI-005。

## 2. Workbench 与 Dock

- `WorkbenchState` 保存窗口本地 active target/panel、layout ID 与 overlays；多窗口不共享焦点或布局状态。
- 五个稳定 `PanelKind` 使用不可变 PanelRegistry 名称：Repository Explorer、Git Log、Commit、Diff、Console。
- 默认布局为中心 Commit/Git Log/Diff tabs、左侧 Repository dock、底部 Console dock；使用 GPUI-Kit 0.9.0 原生拖拽、关闭、缩放和隐藏行为。
- Shell 顶部显示应用名和 “No repository” 占位，底部显示 Ready/No branch；UI-004 将以真实 Session/Snapshot 替换占位。
- OverlayHost 位于最后绘制层，当前不提前创建 Branch/Push 等具体 Overlay。

## 3. 布局持久化与恢复

DockArea dump 包装在 `WorkbenchLayout { schema, layout_id, dock }` 中。只接受当前 schema 与非空 layout ID；DockArea 内部 version 必须匹配 UI-003 layout version，否则回退默认布局。

每次 `DockEvent::LayoutChanged` 增加世代并等待 250ms；只有最后世代执行 dump 和保存 callback。desktop composition root 将文件 IO 放到后台线程，使用同目录临时文件后 rename。平台位置：

- Windows：`%APPDATA%/IdeaGit/workbench-layout.json`
- macOS：`~/Library/Application Support/IdeaGit/workbench-layout.json`
- Linux：`$XDG_CONFIG_HOME/ideagit/workbench-layout.json`，无 XDG 时使用 `~/.config/ideagit`
- 测试/portable override：`IDEAGIT_LAYOUT_PATH`

未知 panel 由 GPUI-Kit DockArea 的 invalid-panel placeholder 保留原始 `PanelState`，后续保存不会静默擦除；已注册 panel 通过 PanelRegistry builder 重建。

## 4. 当前平台实际验证

当前环境：macOS 15.8 arm64、Rust/Cargo 1.99.1。

专项测试覆盖稳定 panel 名、schema/overlay 失败路径、DockArea state envelope、损坏布局回退、文件替换与缺失文件。真实窗口使用同一临时布局路径连续启动两次：首次写出，第二次恢复，均完成创建、绘制和关闭。

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo test -p git-workbench -p idea-shell -p desktop --locked` | 通过；9 个测试全部成功 |
| `cargo clippy -p git-workbench -p idea-shell -p desktop -p desktop-platform --all-targets --locked -- -D warnings` | 通过 |
| 两次 `IDEAGIT_LAYOUT_PATH=... IDEAGIT_UI_SMOKE_DURATION_MS=800 cargo run -p desktop --locked` | 通过；第二次成功恢复首次布局 |
| `cargo check --workspace --locked` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 |
| `cargo test --workspace --locked` | 通过 |
| `RUSTDOCFLAGS='-D warnings' cargo doc -p git-workbench -p idea-shell --no-deps --locked` | 通过 |

## 5. 跨平台与剩余风险

- 不新增第三方包或更新版本，只声明 Workspace 已锁定的 serde/serde_json/internal crates；Cargo.lock 仅同步本地包依赖清单。
- Windows 目标文件替换需要先移除旧 layout 再 rename，进程在该极短窗口崩溃时会回退默认布局，不影响 Git 数据；布局文件不承载 Git 真相。
- 当前未自动化模拟鼠标拖拽像素路径，但使用的是 GPUI-Kit 已测试的 DockArea 原生编辑行为；本提交三平台 Runner 将验证构建和窗口启动，真实 Wayland/Windows 11 拖拽仍保留为平台 QA 风险。
- 实际 Repository Tabs、分支状态、Changes projection、外部 Git edit 到 UI 更新的 M1 E2E 尚未完成，不能据此宣称 §4.7 通过。
