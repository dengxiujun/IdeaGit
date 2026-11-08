# ADR-M4-SELECTION-COMMIT-APPLICATION-001：Selection Commit 应用层装配边界

状态：Draft

日期：2026-11-08

## 问题背景

M4 的底层能力已经具备 `CandidateTreeBuilder`、`PartialCommitCoordinator`、真实 Index reconciliation、Preview 身份校验以及 Changelist 选择模型。当前 `GitCommand::CommitSelection` 仍由 `M2OperationExecutor` 显式返回 `UnsupportedFeature`，UI-013 也保持关闭。

架构契约要求 Selection Commit 经过：

`Entry → Selection → Preview → Validate → Confirm → Execute → Reconcile → Feedback`

并由 Facade 提供 `diff` 与 `prepare_commit`。当前代码缺少将 `ChangeSelection` 与已验证 Diff/Patch、Temporary Index、Candidate Tree、Preview 和 `PartialCommitCoordinator` 串联起来的应用层装配边界。直接在 UI 或 Git CLI 适配器中补齐，会违反 UI → 应用层 → 领域/端口层方向，并可能绕过 `OperationCoordinator`。

## 阻断原因

1. `CandidateTreeBuilder` 只接收已构造的 Patch 字节，没有 DiffReader/完整 Patch 来源端口。
2. `PartialCommitCoordinator` 接收已准备好的事务，不负责从 Selection 解析 Patch、创建 Temporary Index 或建立 Preview。
3. `M2OperationExecutor` 只接收 Index/Commit/Branch 三类 writer，无法安全承载异步的 Selection Commit 准备与确认生命周期。
4. `SelectionCommitRequest` 当前尚无应用层执行器将其绑定到 target、HEAD/Index 指纹、Candidate Tree 和 Journal scope。

## 候选方案

### 方案一：在 UI 中直接调用 CandidateTreeBuilder/PartialCommitCoordinator

不推荐。会让 UI 依赖 Git Core/适配器，绕过 OperationCoordinator 和应用层 Preview/确认边界；也无法统一处理失败、取消和恢复。

### 方案二：在 Git CLI 适配器中增加 Selection Commit 全流程

不推荐。适配器将承担领域编排和 UI 生命周期，形成反向依赖；不同后端无法复用，且核心安全协议无法由应用层统一控制。

### 方案三：新增应用层 SelectionCommitService 与 Diff/Patch 端口，并由 OperationCoordinator 只执行已确认事务

推荐。应用层负责读取 Diff、验证 SelectionFingerprint、构造完整 Patch、创建 Temporary Index、生成 Candidate Tree、建立/失效 Preview 和确认身份；底层端口只负责 Git 进程、临时 Index、提交、Journal 与 reconciliation。`OperationCoordinator` 仍是唯一写操作入口，UI 只调用 Facade。

## 推荐方案

采用方案三，分两层实现：

1. 只读准备层：`DiffReader` + 完整 Patch 证明 + Selection Patch Builder，输出绑定 target、HEAD/Index、selection digest 的 Preview。
2. 执行层：新增 Selection Commit 应用端口/执行器，接收已确认 Preview，使用 Temporary Index、CandidateTreeBuilder 和 PartialCommitCoordinator 完成提交与 reconciliation；任何身份变化都进入重新确认或 RecoveryRequired。

不得把 `CommitSelection` 降级为 `CommitIndex`，不得由 UI 或适配器直接写真实 Index。

## 影响范围

- `git-ports`：补充 Diff/Patch 读取和 Selection Commit 应用边界，复用现有 DTO，不复制领域模型。
- `git-application`：新增 Preview 准备、确认和执行协调服务。
- `git-operations`：增加受 OperationCoordinator 调度的 Selection Commit executor 组合点。
- `apps/desktop`：装配服务，但不在 GPUI 线程执行 Git 进程。
- `git-cli`：仅实现 DiffReader/既有 CandidateTree、Temporary Index、Commit、Journal 和 reconciliation 适配器。
- UI-013：在以上边界及 TEST-004 安全矩阵完成后才开放真实入口。

## 验证标准

1. UI 不直接依赖 Git CLI、CandidateTreeBuilder 或 PartialCommitCoordinator。
2. `CommitSelection` 只能通过 OperationCoordinator 执行，不能降级为 `CommitIndex`。
3. Preview 绑定 target、HEAD/Index、SelectionFingerprint 和 Candidate Tree；过期确认必须拒绝。
4. 真实 Git 仓库覆盖成功、Hook/签名失败、外部 HEAD/Index 竞争、Index reconciliation 失败、崩溃恢复、未选择内容保留、Rename/CRLF/二进制/特殊路径。
5. 在入口开放前，workspace 门禁和 GitHub CI 均通过。

## 批准后动作

批准后更新受影响的 LLD/开发计划与 `PROGRESS.md`，先实现 Diff/Patch 读取端口和应用层准备服务，再实现 OperationCoordinator 装配，最后开放 UI-013 真实入口。
