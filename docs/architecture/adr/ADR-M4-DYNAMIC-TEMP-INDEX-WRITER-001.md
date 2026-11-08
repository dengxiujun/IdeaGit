# ADR-M4-DYNAMIC-TEMP-INDEX-WRITER-001：Partial Commit 动态 Temporary Index 传递

- 状态：Draft
- 日期：2026-11-08
- 范围：M4 UI-013 / CCD-006 / CCD-007

## 问题背景

M4 的 Partial Commit Preview 会为每个 `OperationId` 创建独立的 Temporary Index。确认提交时，Git CLI Commit Writer 必须使用 Preview 绑定的临时 Index，不能使用真实 `.git/index`，也不能使用构造时固定的共享路径。

当前 `GitCliTemporaryIndexCommitWriter` 在构造时把 `GIT_INDEX_FILE` 固定到一个路径，而 `CommitWriter::commit` 只接收 `StandardCommitRequest` 和 `ExecutionContext`，没有接收 `TemporaryIndexLease` 或临时 Index 路径的通道。因此无法把真实 Preview 的动态 Temporary Index 安全地传递到提交边界。

## 阻断原因

如果直接在桌面组合根复用现有 writer，提交可能读取错误的 Index，甚至静默使用真实 Index。这违反 M4 的核心安全约束：未选择内容不得被提交，真实 Index 不得被覆盖或误读。

该问题涉及 `git-ports` 写端口、`git-commit` 协调器和 `git-cli` 适配器边界，不能通过 UI 层临时拼接环境变量绕过。

## 候选方案

### 方案一：扩展 CommitWriter 请求，显式携带 Temporary Index 身份

将 Partial Commit 专用写端口改为接收带 `TemporaryIndexLease` 的请求，或新增 `PartialCommitWriter` 端口；Coordinator 在调用时传递事务中已验证的 lease。Git CLI 适配器每次调用使用该路径，并校验 lease 的 operation identity。

优点：身份显式、类型安全、最容易验证不使用真实 Index；普通 `CommitWriter` 保持 M2 语义不变。

缺点：新增端口/请求类型，需要更新 Coordinator、适配器和组合测试。

### 方案二：向 ExecutionContext 增加受限的 Temporary Index 元数据

由 Coordinator 将临时 Index 路径放入 ExecutionContext，Git CLI writer 从 context 读取。

优点：改动表面较小。

缺点：路径元数据容易被普通写操作误用；类型边界弱，难以证明普通 Commit 不会意外使用临时 Index；需要定义并发、生命周期和清理语义。

### 方案三：Coordinator 直接持有动态 writer 工厂

每次提交前由 Coordinator 根据事务 lease 创建 writer 实例，writer 构造时绑定该路径。

优点：复用现有固定路径 writer 的实现。

缺点：Coordinator 需要依赖 writer 工厂抽象，事务引擎与适配器组合更复杂；仍需防止工厂忽略 lease，并增加生命周期验证。

## 推荐方案

推荐方案一：新增 Partial Commit 专用写端口或显式请求，携带已验证的 `TemporaryIndexLease`，并要求 `transaction_id == operation_id`。普通 M2 CommitWriter 不改变。这样可以在类型边界上证明 Partial Commit 使用的是 Preview 的动态 Temporary Index，且不会静默回退到真实 Index。

## 影响范围

- `crates/core/git-ports`：新增或调整 Partial Commit 写端口/请求。
- `crates/core/git-commit`：Coordinator 在 CommitInProgress 阶段传递并校验 lease。
- `crates/adapters/git-cli`：按请求动态设置 `GIT_INDEX_FILE`，拒绝缺失或身份不匹配的路径。
- `apps/desktop`：生产组合根改用动态 Partial Commit writer。
- 测试：真实 Git 仓库覆盖动态路径、真实 Index 保持不变、跨 operation 拒绝、失败恢复和临时文件清理。

不更换 Git CLI、Rust 依赖或 Git 数据模型；普通 M2 Commit 路径不应受到影响。

## 验证标准

1. 每个 Prepared Transaction 使用其自身 Temporary Index，两个并发 operation 不交叉。
2. writer 缺失 lease、lease operation 不匹配或路径已改变时，在 Git 进程启动前拒绝。
3. 真实 Git 集成测试证明 Partial Commit 后真实 `.git/index` 字节保持预期，未选择内容不进入 HEAD。
4. Hook、签名失败、外部 Index 竞争和 Coordinator RecoveryRequired 语义保持不变。
5. M2 普通 Commit 的现有端口、测试和行为不变。

## 批准记录

待项目负责人批准后实施推荐方案。
