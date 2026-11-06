# IdeaGit Agent 开发规范

## 1. 项目

IdeaGit 是一款生产级、跨平台的 Git 桌面客户端。

- 编程语言：Rust
- Edition：2024
- UI：GPUI + GPUI-Kit
- Git 后端：系统 Git CLI
- 支持平台：Windows、macOS、Linux
- UI 参考：IntelliJ IDEA 2026.x 新版界面

## 2. 权威文档

开始实现任务前，请阅读相关文档：

- docs/architecture/ARCH-001-overview.md
- docs/architecture/LLD-GIT-CORE-001.md
- docs/architecture/LLD-UI-001.md
- docs/architecture/LLD-CCD-001.md
- docs/architecture/TECH-BASELINE-001.md
- docs/plans/IP-001-development-plan.md

如果文档之间存在冲突，请在实现前报告冲突。
不得在未说明的情况下擅自修改架构决策。

## 3. 架构

依赖方向：

UI -> 应用层 -> 领域层 / 端口层
适配器实现端口。

规则：

- git-domain 不得依赖 GPUI。
- UI 不得直接调用 Git CLI。
- Git 写操作必须使用 OperationCoordinator。
- 后端适配器不得依赖 UI。
- 避免循环 crate 依赖。
- 保持 Git 路径语义和对象身份语义不变。

## 4. Git 安全

绝不能静默丢弃用户修改。

- 未经验证，绝不能覆盖真实的 Git 索引。
- 不得在用户仓库中使用 `git reset --hard`。
- 未经用户明确批准，不得强制推送。
- 对结果不确定的操作，不得自动重试。
- 保留未暂存修改和未选择的修改。
- 写操作必须使用真实 Git 仓库测试。
- 将外部 Git 进程视为可能并发写入者。

## 5. 实现工作流

对于每个开发任务：

1. 阅读相关规范。
2. 检查现有代码和 API。
3. 说明实现计划。
4. 确定需要修改的文件。
5. 仅实现请求范围内的内容。
6. 添加单元测试或集成测试。
7. 运行相关检查。
8. 检查差异。
9. 报告结果和剩余风险。

除非确实运行过测试，否则不得声称测试已通过。

## 6. Rust 质量

- 优先使用强类型，而不是原始字符串。
- 使用结构化错误。
- 生产路径中避免使用 `unwrap()` 和 `expect()`。
- 未作出充分说明，不得引入不安全 Rust。
- 不得阻塞 GPUI UI 线程。
- 使用有界的异步并发。
- 保持取消和恢复语义。

## 7. 必需检查

适用时执行：

```text
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

如果某条命令无法执行，请说明原因。

## 8. 完成定义

只有满足以下条件，任务才算完成：

- 实现符合相关 LLD。
- API 已有文档说明。
- 测试覆盖正常路径和失败路径。
- 现有测试通过。
- 已考虑跨平台影响。
- 未修改无关文件。
- 最终响应包含修改文件、测试结果、风险和后续事项。

## 9. 任务范围

一次只实现一个已跟踪的开发任务。

不得实现无关功能。
未经授权，不得进行大范围重构。
更新依赖前必须说明原因。

## 10. 语言

使用简体中文说明实现决策、计划、评审和测试结果。
保持 Rust 标识符和代码注释清晰、一致。
