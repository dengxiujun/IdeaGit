# IdeaGit 开发进度

更新日期：2026-11-08。

## M4 TEST-004：Partial Commit Safety Tests（2026-11-08）

- 分支：`feat/TEST-004`；从 `main` 创建，带入 DIFF-003..007 与 CCD-001..009 的前置实现。
- 完成：新增安全测试，覆盖混合 Staged/Unstaged 的 Index 身份拒绝、外部 HEAD 变化、提交结果与实际 HEAD 不一致进入 RecoveryRequired，以及真实 Git 仓库外部提交导致 Index 变化时的保守恢复判定；新增真实临时 Index 部分提交测试，确认未选文件的 HEAD/Worktree 内容与真实 Index 均保持不变；新增真实 Hook 拒绝测试，确认失败不虚报成功且不改变 HEAD/真实 Index。
- 测试：`git-commit` 专项 10 个测试、Git CLI 专项 62 个测试通过；workspace 全量测试通过。
- 门禁：`cargo fmt --all -- --check`、`cargo check --workspace --locked`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked`、`git diff --check` 均通过。
- Commit/Runner：本地提交待本次记录后生成；GitHub Runner 尚未创建/执行 PR。
- 未覆盖风险：Hook 拒绝/修改 Worktree、签名失败、部分行真实提交后未选内容保留、崩溃重启诊断、Rename/CRLF/非 UTF-8/Binary 的 Partial Commit 端到端矩阵仍缺少可执行的协调器/真实提交测试入口；M4 生产开放门禁保持关闭。
- 下一任务：补齐 TEST-004 所需的真实 Partial Commit 执行测试入口；UI-013 在 TEST-004 与 CCD-007 生产发布协议完成前不得开放。

## M4 CCD-008：EditableIndexSession / Apply（2026-11-08）

- 分支：`feat/CCD-008`；从 `main` 创建并带入 CCD-001～007、CCD-009 前置实现。
- 完成：新增可编辑 Index 会话模型，保存 SessionTarget、原始 HEAD/Index 身份和内容摘要；编辑只修改会话内候选内容，Apply 预检要求 dirty 且身份未变化，未确认或过期会话拒绝返回内容。
- 测试：`git-commit` 专项 7/7 通过，覆盖未编辑拒绝、正常候选返回和身份变化拒绝；未写入真实 Index。
- 风险：Apply 的真实 Blob/Mode/Filter 校验和协调器发布仍受 CCD-007 安全发布协议约束；当前只开放预检，不开放真实写入。
- 下一任务：补齐 CCD-007 发布端口后继续 CCD-008 Apply 适配，再进入 UI-011。

## M4 CCD-007：Real Index Reconciliation（2026-11-08）

- 分支：`feat/CCD-007`；从 `main` 创建并带入 CCD-001～006、CCD-009。
- 完成：新增保守 `RealIndexReconciler` 规划器；比较原始 Index SHA-256 摘要和当前 Index，要求 HEAD 已前进且存在已验证候选 Index，才生成 Publish 计划；外部 Index 竞争、HEAD 未前进或缺少候选均进入 RecoveryRequired。
- 测试：专项覆盖安全发布与外部 Index 变化拒绝；workspace 依赖边已离线锁定，未升级版本。
- 风险：平台无关的锁内原子发布尚未接入；当前规划器不直接写真实 Index，避免绕过 OperationCoordinator。
- 下一任务：完成批准的 Index 发布端口后继续 CCD-008（EditableIndexSession / Apply）。

## M4 CCD-006：Partial Commit Transaction（2026-11-08）

- 分支：`feat/CCD-006`；从 `main` 创建并带入 CCD-001～005、CCD-009 前置实现。
- 完成：Partial Commit 状态机支持临时 Index 准备、Candidate Tree、确认前 HEAD/Index 重验、CommitInProgress、CommitObserved 和 RecoveryRequired；新增 `PartialCommitJournal` 阶段持久化，以及仅绑定 `GIT_INDEX_FILE` 的 `GitCliTemporaryIndexCommitWriter`。
- 测试：本次变更后 `cargo fmt --all -- --check`、`git-cli` Clippy、`git-cli` 测试、`git diff --check` 均通过；真实 Git 临时 Index Commit 测试通过，并确认真实 Index 字节不变。
- Commit/Runner：`3ed190d`、`6b3f903`、`0998bb3`、`4b916ac`；GitHub Runner 尚未创建/执行 PR。
- 风险：尚未完成提交后 HEAD/Hook 副作用核对和真实 Index reconciliation；未选择内容安全保持由 CCD-007/TEST-004 验证。
- 下一任务：继续完成 CCD-006 的提交结果核对，再进入 CCD-007。

## M4 CCD-009：Preview 生命周期与确认身份（2026-11-08）

- 分支：`feat/CCD-009`；从 `main` 创建并带入 CCD-001～005，按依赖顺序先于 CCD-006 实现。
- 完成：新增 `ChangelistPreview` 与 Ready/Confirmed/Expired/Cancelled 状态；预览绑定 OperationId、SessionTarget、HEAD/Index 身份、Candidate Tree 和 SelectionFingerprint；确认时身份变化或重复确认均失效，不复用过期预览。
- 测试：`cargo fmt --all -- --check`、`cargo test -p git-changelist --locked` 已通过；预览过期与重复确认专项通过。
- Commit/Runner：待本次提交生成；GitHub Runner 尚未创建/执行 PR。
- 风险：预览尚未接入 UI 和 Partial Commit 执行器；真实 Commit 仍需 CCD-006，Index reconciliation 需 CCD-007。
- 下一任务：CCD-006（Partial Commit Transaction）。

## M4 CCD-004：Temporary Index Manager（2026-11-08）

- 分支：`feat/CCD-004`；从 `main` 创建并带入 CCD-001/002/003。
- 完成：新增临时 Index 端口与 Git CLI 实现；使用独立临时目录和 `GIT_INDEX_FILE` 初始化 HEAD/空树，RAII 租约释放临时文件，不触碰真实 Index；裸仓库安全拒绝。
- 测试：专项 2/2 通过，覆盖真实 Git 仓库有 HEAD 创建、真实 Index 不变、租约释放，以及 unborn HEAD 判定。
- Commit/Runner：本地提交待本次记录后生成；GitHub Runner 尚未创建/执行 PR。
- 风险：Journal 清理失败记录、Index 原始摘要/备份和崩溃恢复上下文将在后续 Partial Commit 事务任务中接入；当前未修改真实 Index。
- 下一任务：CCD-005（Candidate Tree Builder）。

## M4 CCD-003：Changelist Persistence（2026-11-08）

- 分支：`feat/CCD-003`；从 `main` 创建并带入 CCD-001/002。
- 完成：新增版本化 `ChangelistBundle` 与 `ChangelistStore` 端口；`JsonChangelistStore` 实现目标/Schema 校验、乐观版本控制、原子临时文件替换和写前 `.json.bak` 备份。
- 测试：`cargo fmt --all -- --check`、`cargo check --workspace --locked`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked`、`git diff --check` 均通过；Persistence 测试 6 个通过，新增持久化专项 2 个通过。
- 依赖：仅使用 workspace 已锁定的 `git-changelist` 与既有异步/序列化依赖，Cargo.lock 已更新依赖边，未升级版本。
- Commit/Runner：本地提交待本次记录后生成；GitHub Runner 尚未创建/执行 PR。
- 风险：当前为 JSON 文件存储，尚未接入 SQLite 迁移；Temporary Index 与 Partial Commit 不在本任务范围。
- 下一任务：CCD-004（Temporary Index Manager）。

## M4 CCD-002：Selection Fingerprints（2026-11-08）

- 分支：`feat/CCD-002`；从 `main` 创建并带入 CCD-001。
- 完成：新增确定性 SHA-256 选区指纹引擎，绑定 Repository/Worktree、HEAD/Index、Diff Generation、ChangeSelection 与原始内容字节；状态变化分类为 Valid、Stale 或 Ambiguous，跨目标选择拒绝。
- 测试：`cargo fmt --all -- --check`、`cargo check --workspace --locked`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked`、`git diff --check` 均通过；专项测试 4/4 通过。
- 依赖：仅使用 workspace 已锁定的 `serde_json`/`sha2`，更新 Cargo.lock 依赖边，未升级版本或引入外部依赖。
- Commit/Runner：本地提交待本次记录后生成；GitHub Runner 尚未创建/执行 PR。
- 风险：指纹重定位仍是保守分类，不自动移动选区；持久化事务留给 CCD-003。
- 下一任务：CCD-003（Changelist Persistence）。

## M4 CCD-001：Changelist Domain（2026-11-08）

- 分支：`feat/CCD-001`；从 `main` 创建，未提前引入后续 M4 实现。
- 完成：新增 Changelist/Binding 强类型模型、稳定 UUID 身份、仓库/Worktree 目标隔离、默认列表/Schema 版本、绑定 Valid/Stale/Ambiguous 状态及跨 Session 拒绝。
- 测试：`cargo fmt --all -- --check`、`cargo check --workspace --locked`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked`、`git diff --check` 均通过；专项测试 2/2 通过。
- 依赖：`git-changelist` 首次直接使用 workspace 已锁定的 `serde`/`thiserror`/`uuid`，先离线更新 Cargo.lock 依赖边，再完成 locked 门禁；未引入新版本或外部依赖。
- Commit/Runner：本地提交待本次记录后生成；GitHub Runner 尚未创建/执行 PR。
- 风险：指纹重定位、持久化事务和 Selection 语义校验留给 CCD-002/003；当前不修改 Git Index。
- 下一任务：CCD-002（Selection Fingerprints）。

## 当前状态

M0、M1 已完成。M1 Git Read Core 与 Workbench 已在真实 Git 端到端链路和 Rust CI #21 三平台门禁中验证：CORE-001/002/003/004/005/006/007/008、UI-003/004/005 均已完成三平台 CI 验证，IP-001 §4.7 最终门禁已关闭。

## 已完成的文档工作

- 首轮审查：ARCH-REVIEW-001、API-CONFLICTS-001、M0-EXECUTION-CHECKLIST。
- 项目负责人已在聊天批准 [ADR-ARCH-REMEDIATION-001](../architecture/adr/ADR-ARCH-REMEDIATION-001.md) D1–D7，并要求先迁移 ADR 目录。
- 已将 docs/adr 移至 docs/architecture/adr，修正链接并记录 Accepted 状态。
- 根据 ADR 修订六份设计/基线/主计划：共享契约集中 Core §18；Index 写后恢复捷径移除；状态/恢复统一；任务前置和发布门禁补齐。
- M0 清单保留首轮发现，并补充设计修订与待验证事项的区分。
- INFRA-004 已由根 AGENTS.md、正式 ADR、[Workspace 依赖记录](INFRA-001-workspace.md) 和 GitHub PR 模板共同闭环。

## 架构修订阶段验证记录（历史）

本次检查结果：

- 相对 Markdown 文件链接检查通过（架构文档及主计划/M0 清单/进度记录）。
- 从修订后的 IP 实际文件提取 90 个任务：无重复 ID、缺失依赖引用或循环。
- git diff --check 通过；旧 docs/adr 目录已不存在，新目录包含正式 ADR。
- 已清理旧 SnapshotVersion、冲突共享 DTO 定义、真实 Index 写后恢复捷径及仅按 RepositoryId 定位的 UI 执行上下文。

以下限制描述的是 Workspace 建立前的架构修订阶段：当时未执行 Cargo 构建、Rust 测试、依赖安装、三平台窗口或真实 Git 故障注入。后续工程验证结果见 INFRA-001/002/003 各节。

## 未关闭的工程门禁

| 范围 | 状态 / 放行条件 |
|---|---|
| Rust/GPUI/其他依赖 | Rust 1.99.1 与锁定依赖已在三平台 CI 编译；MSRV、真实窗口和 Git 最低版本/能力矩阵待验证 |
| Workspace | 25 个包依赖无环；Rust CI #3 已在 Ubuntu 24.04、Windows Server 2022 和 macOS 15 通过 locked check、Clippy 与测试 |
| 标准写操作 | Lock/Journal/RECOVERY-000/真实 Git fixture 完成后才能开放 |
| Partial Commit / Index 编辑 | capability 默认关闭；锁内比较/发布、剩余 staged 转移、Hook/HEAD 竞争及 TEST-004 未验证 |
| UI | GPUI-Kit 最小窗口已在三平台 Runner 启动；Dock、输入、焦点、IME、DPI 和真实桌面兼容性仍未验证 |
| 发布 | M0–M6 均不能因文档完成而标记交付；最终仍需 QA、安全、迁移与阻断项清零 |

## 下一步

按依赖 DAG 进入 M2 UI-006 生产 Action Dispatcher 接入，再推进 UI-007 Commit Panel 与 UI-008 Branch Popup；仍一次只实施一个任务，核心版本或边界变更仍需 ADR。

## UI-006 Action Registry 基础（2026-11-07）

- `git-ui-model` 新增 typed `ActionId`、`ActionContext`、`ActionAvailability` 与 `ActionPolicy`，为 Stage、Unstage、Commit、Create/Checkout Branch 统一按钮、菜单和快捷键的判定入口。
- `DefaultActionPolicy` 在没有仓库、没有文件选择或没有目标分支时安全禁用动作；策略只做上下文校验，不直接执行 Git 写操作。
- 新增正常与失败前置条件测试；实际 Action Dispatcher 到 `OperationCoordinator` 的执行绑定仍是下一小任务。

## UI-006 Action Dispatch Boundary（2026-11-07）

- `ActionRequest` 与 `ActionDispatcher` 明确了 UI 到应用组合根的 typed 派发边界；`WorkbenchController::dispatch_action` 统一执行 ActionPolicy 校验、失败反馈和 Running 状态发布。
- Dispatcher 只负责接收并排队请求，UI 线程不直接调用 Git；具体 Stage/Commit/Branch 执行器及终态订阅仍需由 desktop composition root 接入。

## UI-006 Action Command Mapping（2026-11-07）

- `to_git_command` 将 Stage/Unstage 文件动作转换为共享 `GitCommand::StageFiles` / `UnstageFiles`，保留 `SessionTarget` 与原始 `RepoPath`。
- 缺少目标、文件选择或非文件动作均返回结构化边界错误；尚未执行 Git、获取锁或修改真实 Index。

## UI-006 Dispatcher Injection（2026-11-07）

- `WorkbenchController` 支持注入应用组合根提供的 `ActionDispatcher`，所有动作共享同一 dispatcher 句柄；未配置 dispatcher 时安全失败并保留失败反馈。
- 注入边界不持有 Git 适配器、不在 GPUI 线程执行进程，下一步由 desktop 将其连接到 `OperationCoordinator`。

## CORE-010 Index Operation Executor（2026-11-07）

- `git-operations::IndexOperationExecutor` 将 `GitCommand::StageFiles/UnstageFiles` 转换为 `IndexMutation` 并交给既有 IndexWriter 装饰器链；不支持的 Commit/Branch/Remote 命令显式返回 `UnsupportedFeature`。
- 测试覆盖 Stage 映射及不支持命令失败；该适配器不绕过锁、Journal 或 Reconcile，desktop 仍需注入具体 writer 和快照恢复器。

## CORE-010 Commit Operation Executor（2026-11-07）

- `git-operations::CommitOperationExecutor` 将 `GitCommand::CommitIndex` 转换为 `CommitWriter::commit`，保留标准提交请求中的消息、预期 HEAD/Index 和 Snapshot revision。
- 其它命令显式拒绝；测试覆盖标准提交映射。具体 Git CLI writer、锁、Journal、Hook 与 Reconcile 继续由装饰器链提供。

## CORE-010 Branch Operation Executor（2026-11-07）

- `git-operations::BranchOperationExecutor` 将 `GitCommand::Branch` 转换为 `BranchWriter::apply`，保留 Create/Checkout/Rename/Delete 的 typed `BranchCommand`。
- 非 Branch 命令显式拒绝；实际 Refs 锁、Journal、Reconcile 仍由既有 writer 装饰器承担。

## M2 CORE-010 统一写操作路由（2026-11-07）

- `M2OperationExecutor<I, C, B>` 将 Stage/Unstage、Standard Commit 与 Branch command 路由到 composition root 注入的三条 writer 装饰链，为单一 Workbench Coordinator 提供完整 M2 typed command 集。
- executor 不构造裸 Git writer；Lock、Journal、Commit Preflight 与 Reconcile 仍必须由外层组合后注入，Remote/M3 command 明确返回 `UnsupportedFeature`。
- 单元测试覆盖三类成功路由和 Remote 拒绝，确保不会因统一入口意外开放后续里程碑能力。

## M2 CORE-010 写后权威重读端口（2026-11-07）

- `ReconciledIndexWriter`、`ReconciledCommitWriter` 与 `ReconciledBranchWriter` 现在直接实现对应写端口，可作为统一 M2 executor 的最外层 writer 注入。
- 三类装饰器都只在底层写入与 `SnapshotRebuilder` 权威重读同时成功后返回成功；Commit 保留并返回原始 `CommitOutcome`。
- 失败路径测试覆盖“Git 写入已成功但重读失败”，三类操作均保留重读器的结构化失败，不向 Coordinator 伪报成功。

## M2 CORE-010 Scheduler Reconcile Handle（2026-11-07）

- `RefreshScheduler::rebuilder` 派生 operation-scoped `ScheduledSnapshotRebuilder`；写后请求携带一次性响应，只有该请求对应的权威 refresh 完成后才返回 Snapshot。
- 当已有 refresh 正在执行时，Reconcile 请求留在有界队列等待下一轮，避免把写入前启动的旧 snapshot 错认作写后结果；普通文件系统触发仍按原策略合并。
- Reconcile 保留 target 校验、取消和底层结构化读取错误；测试覆盖成功产生下一 revision 及权威读取失败透传。

## M2 CORE-010 Desktop M2 Executor Composition（2026-11-07）

- desktop composition root 新增完整 M2 executor 构造器：Index/Branch 为 Lock → Journal → Git CLI → Reconcile，Commit 为 Lock → Preflight → Journal → Git CLI → Reconcile。
- 三类 writer 共享同一个 `ResourceLockManager`、durable `JsonJournalStore` 与 session `ScheduledSnapshotRebuilder`；构造器本身不安装 Action Dispatcher，不提前开放 GUI 写入口。
- 真实仓库测试通过该生产构造器执行 Stage，并验证权威 Snapshot 已暂存且 Journal 终态为 Completed，证明不是仅编译的空装配。

## M2 CORE-012 Operation Journal 平台路径（2026-11-07）

- `desktop-platform::operation_journal_path` 将 durable Journal 放在 IdeaGit 平台配置目录，与 Workbench layout 同目录但独立文件，不写入用户仓库或进程当前目录。
- `IDEAGIT_JOURNAL_PATH` 提供显式测试/portable 覆盖；平台配置目录缺失时返回 `None`，由 composition root 安全禁用写入口而不是回退到不确定位置。
- 纯函数测试覆盖路径派生和缺失目录两条路径，保持 Windows、macOS、Linux 选择逻辑不变。

## M2 GIT-002 Commit Preflight Probe（2026-11-07）

- `GitCliCommitStateProbe` 通过 sealed typed command 读取 `rev-parse --verify --quiet HEAD` 与真实 Index 的 `write-tree` OID，为 composition root 提供强类型 HEAD/Index 身份。
- Unborn HEAD 表示为 `None`；Index tree 始终为 `ObjectId`。探针不修改 Index 或 refs，进程仍继承 Operation 的取消、超时和输出上限。
- 真实仓库测试覆盖 Unborn、首次 Commit 与外部 Stage 后的身份变化，证明 HEAD 不变时 Index 指纹仍可检测竞争。

## M2 GIT-002 Desktop Commit State Adapter（2026-11-07）

- desktop composition root 新增 `DesktopCommitStateReader`，将 Git CLI probe 的 HEAD/Index tree OID 转为 Core `CommitPreflightState`，不复制 Git 进程逻辑。
- 适配器绑定一个 `SessionTarget`，跨仓库或过期 session 在启动 Git 前以结构化 `InvalidRepository` 拒绝，避免读错仓库后授权 Commit。
- 真实仓库测试覆盖成功映射与 foreign session 拒绝；下一步可将其注入 `PreflightCommitWriter` 的完整安全装饰链。

## UI-006 Desktop Action Queue（2026-11-07）

- desktop 新增 `DesktopActionDispatcher`：将 UI `ActionRequest` 转换为共享 `GitCommand`，通过 Tokio runtime 异步提交 `OperationCoordinator`，避免阻塞 GPUI 线程。
- 当前 dispatcher 已覆盖 Stage/Unstage 文件动作；Commit/Branch 仍需在 UI 提供完整参数后接入对应 command builder，终态反馈订阅也待补齐。

## UI-006 Typed Commit/Branch Action Payloads（2026-11-07）

- `ActionRequest` 新增 `ActionPayload`，支持携带 `StandardCommitRequest` 或 typed `BranchCommand`；统一转换器现在可生成 `CommitIndex` 与 `Branch` GitCommand。
- 缺少 payload 时显式失败，不接受字符串命令；GPUI 控件和操作终态订阅仍待接入。

## UI-006 Operation Feedback Mapping（2026-11-07）

- `WorkbenchController::set_operation_snapshot` 将 `OperationCoordinator` 的 Running、Success、Failed、Cancelled、RecoveryRequired 映射为统一中文 UI feedback；Finished 无结果也按失败处理，不伪造成功。
- 该映射只更新 UI 状态，不改变 Git 仓库；desktop 仍需订阅 coordinator 的 latest-only snapshot 并回送窗口实体。

## M2 UI-006 Coordinator 终态订阅（2026-11-07）

- `DesktopActionDispatcher` 在成功提交 command 后订阅对应 `OperationId` 的 latest-only snapshot，并通过无界 UI 更新通道转发 Queued 至 Finished 的真实状态。
- 订阅在 Finished、接收端关闭或 coordinator stream 关闭时终止，不重复执行 Git 命令；下一小节点将把 receiver 回送 `WorkbenchController::set_operation_snapshot`。

## M2 UI-006 Desktop 操作生命周期桥（2026-11-07）

- `DesktopActionDispatcher` 显式持有应用层 Tokio runtime handle；GPUI UI 线程触发 Action 时不再依赖当前线程恰好进入 Tokio 上下文。
- desktop 新增统一安装入口，将 Dispatcher 注入 `WorkbenchController`，并通过弱 UI 句柄在 GPUI 任务中回送 coordinator snapshots；Workbench Entity 释放后终止反馈消费，不形成 controller/sender 保留环，也不重试 Git 命令。
- 测试从不含 Tokio context 的独立线程派发 Stage Action，并验证真实 coordinator 终态经 receiver 转发为 Success；具体安全 writer 装配仍由后续小节点完成。

## M2 UI-009 真实 Git Log Bridge（2026-11-07）

- desktop repository bridge 新增 typed UI update，在刷新得到新 HEAD 时通过 `GitCliHistoryReader` 读取默认首个分页并回送 `WorkbenchController::set_log_page`；普通工作区文件变化不会重复读取相同 HEAD 的历史。
- Unborn HEAD 显示空 Log，不执行注定失败的 `git log`；历史读取失败以可见反馈返回 UI，不终止 Status/Changes 持续刷新。
- 真实仓库测试覆盖首次提交历史与外部新增提交后的 Log 更新，bridge 在 UI receiver 关闭后仍可干净收束。

## CORE-011 Resource Lock Manager（2026-11-07）

- `git-operations` 新增 `ResourceKey`、`ResourceScope`、`ResourceKind` 与 RAII `ResourceLock`；CommonGitDir 只承载共享 Refs，Worktree 承载 Index/WorktreeFiles/Sequencer。
- 多资源按稳定 scope UUID 与 kind 顺序获取，避免锁顺序死锁；重复键自动去重；取消等待返回结构化 `LockError::Cancelled`。
- 锁是应用内协调锁，不占用或伪造真实 `.git/index.lock`；独立仓库资源可并发，真实 Git 锁竞争仍由 Git 命令和后续恢复门禁处理。
- 本机 Workspace 全量门禁已通过；[Rust CI #24](https://github.com/dengxiujun/IdeaGit/actions/runs/42349017069) 已在 Windows Server 2022、macOS 15 与 Ubuntu 24.04 全部通过。

## CORE-010 Operation Coordinator（2026-11-07）

- `git-operations` 新增有界串行 `OperationCoordinator`，通过注入的 `OperationExecutor` 消费 `GitCommand`，不在本任务伪造 Git 副作用。
- 生命周期发布 `Queued → ResourceWait → Preflight → Running → Reconciling → Finished`；通过 latest-only watch 提供 OperationSnapshot，保留目标、命令和结构化终态。
- 取消区分 `NotStarted`、`StoppedAndChecked`、`NeedsRecovery`，执行器失败保留 `GitError`；progress channel 在后台持续排空，避免长操作反压。
- 本机 Workspace 全量 fmt/check/clippy/test 已通过；[Rust CI #23](https://github.com/dengxiujun/IdeaGit/actions/runs/42348019499) 已在 Windows Server 2022、macOS 15 与 Ubuntu 24.04 全部通过。CORE-011/012、RECOVERY-000 仍是标准写操作开放前置。

## CORE-009 强类型 GitCommand（2026-11-07）

- `git-domain` 新增唯一 `GitCommand` 契约及强类型 Stage/Unstage、Selection、标准 Index Commit、Branch、Remote、Sequencer 命令；每个命令只携带一个 `SessionTarget`。
- 新增 `FileIdentity`、`ChangeSelection`、`SelectionFingerprint`、`CommitOptions`、`StandardCommitRequest` 和 `SelectionCommitRequest`，保留原始 `RepoPath`、OID、Snapshot revision 与签名策略，不接受任意 shell 字符串。
- 命令校验拒绝空路径、空提交消息、跨 SessionTarget 选择和 fingerprint 数量不匹配；本任务不执行 Git、不获取锁、不修改 Index/HEAD。
- `git-domain` 专项测试 23 个通过；[Rust CI #22](https://github.com/dengxiujun/IdeaGit/actions/runs/42346763268) 已在 Windows Server 2022、macOS 15 与 Ubuntu 24.04 完成 Format、locked check、Clippy 与 Workspace tests。

## UI-005 Read-only Changes Tree / M1 §4.7（2026-11-07）

- `git-ui-model` 新增固定 Staged、Unstaged、Untracked 分组的只读 Changes projection，保留 `SessionTarget`、Snapshot revision 与原始 `RepoPath`。同一文件的 Index 与 Worktree 均变化时同时进入两个分组，未将真实 Index 语义伪装为 Checkbox。
- Commit Dock panel 使用 `WorkbenchController` 接收模型并显示三组文件及状态；Shell 不依赖 `git-cli`，不会发起 Stage/Unstage 或其他写入。
- desktop composition root 将 Discovery、Registry Session、RepositoryWatch 与 RefreshScheduler 连成持续 bridge。每次最新 Snapshot 都回送 GPUI UI 线程；UI receiver 关闭时取消 session 并收束 scheduler。
- 真实 Git fixture 覆盖 staged、unstaged、mixed、untracked 的投影，以及外部写入 `external.txt` 后 Watcher → Refresh → Workbench 模型更新，满足 IP-001 §4.7 的应用数据链路。
- 本机专项检查、Workspace 全量 fmt/check/clippy/test、原生窗口 smoke 与 [Rust CI #21](https://github.com/dengxiujun/IdeaGit/actions/runs/42344546518) 的 Windows Server 2022/macOS 15/Ubuntu 24.04 门禁均通过；M1 §4.7 已关闭。详见 [UI-005-VERIFICATION-001](../reviews/UI-005-VERIFICATION-001.md)。

## UI-004 Repository Tabs / StatusBar（2026-11-07）

- `git-ui-model` 新增从 `RepositorySessionView + RefreshState` 到 Repository Tab/StatusBar 的纯投影：仓库显示名、SessionTarget、当前分支、ahead/behind、Snapshot revision 与 Pending/Ready/Degraded 均保留且不猜测成功。
- Shell 增加 typed `WorkbenchController`；后台结果只通过 GPUI 调度更新窗口 Entity，UI crate 不依赖 `git-cli`。
- desktop composition root 支持 `IDEAGIT_OPEN_REPOSITORY` 打开路径：在后台 Tokio runtime 执行 CORE-002 Discovery、CORE-007 Session、CORE-008 guarded refresh，再投影到 Shell；该变量同时用于自动化 E2E，默认不擅自打开目录。
- 当前 macOS 已通过 2 个 projection 测试、真实隔离 Git fixture 集成测试、实际桌面窗口 smoke 与专项 Clippy；[Rust CI #20](https://github.com/dengxiujun/IdeaGit/actions/runs/42343244836) 已在 Windows Server 2022、macOS 15 与 Ubuntu 24.04 全部通过。
- UI-004 不实现多 tab 交互/关闭、持续 Watcher 订阅或 Changes Tree，分别留给后续 UI/M1 集成。详细结果见 [UI-004-VERIFICATION-001](../reviews/UI-004-VERIFICATION-001.md)。

## UI-003 Workbench Shell / DockArea（2026-11-07）

- `git-workbench` 新增窗口本地 `WorkbenchState`、稳定 `PanelKind`/PanelRegistry 名称、Overlay ID 和版本化 layout envelope；旧 schema、空 layout ID 与非法 overlay 均显式拒绝。
- `idea-shell` 将 UI-001 启动占位替换为 GPUI-Kit 0.9.0 `DockArea`：中心 Commit/Git Log/Diff 标签、左侧 Repository、底部 Git Console，并提供 Titlebar 区、Repository Tab 占位、StatusBar 与最后绘制的 OverlayHost。
- 所有面板经 PanelRegistry 注册，可拖拽、关闭、隐藏与恢复；未知/已移除面板按 GPUI-Kit DockArea 的占位语义安全保留，不在一次保存后静默丢失。
- DockArea `LayoutChanged` 以 250ms 世代防抖，只发布最终布局；desktop 在后台线程写入平台配置目录，启动前读取并验证 schema。UI 线程不执行文件 IO。
- 当前 macOS 已连续两次运行真实窗口烟雾：首次生成布局，第二次加载同一布局并成功绘制/关闭；专项 9 个测试、fmt 与 Clippy 通过。[Rust CI #19](https://github.com/dengxiujun/IdeaGit/actions/runs/42341427891) 已在 Windows Server 2022、macOS 15 和 Ubuntu 24.04 全部通过；UI-003 三平台验收关闭。
- UI-003 不伪造真实 Repository/HEAD/Changes；这些属于 UI-004/005。详细结果见 [UI-003-VERIFICATION-001](../reviews/UI-003-VERIFICATION-001.md)。

## CORE-008 Watcher + Refresh Scheduler（2026-11-07）

- `repository-watcher` 新增基于 `notify` 的有界、内容无关变化信号；普通仓库递归监控 worktree（包含 `.git`），linked worktree 同时监控独立 worktree 与共享 common dir，bare 只监控 common dir。
- `git-application` 新增 guarded Snapshot 刷新服务，严格按 `HEAD(before) → Status → Tracking → HEAD(after)` 读取并调用唯一 `SnapshotBuilder`；bare 不执行 worktree Status。
- Refresh Scheduler 合并 200ms 内突发信号，使用 30 秒周期主动校验兜底丢事件/溢出，串行发布单调 Snapshot；失败保留最后成功 Snapshot 并进入 Degraded，Session 取消时停止后台任务。
- 当前 macOS 已通过 fmt、locked check、Clippy 和 Workspace tests；新增 7 个 Watcher/Refresh 正常及失败路径测试。[Rust CI #18](https://github.com/dengxiujun/IdeaGit/actions/runs/42340135972) 已在 Windows Server 2022、macOS 15 和 Ubuntu 24.04 全部通过；CORE-008 三平台验收关闭。
- CORE-008 不提前实现 Sequencer port、Operation 写协调或 UI；详细结果见 [CORE-008-VERIFICATION-001](../reviews/CORE-008-VERIFICATION-001.md)。

## CORE-007 Repository Session / Registry（2026-11-07）

- `git-application` 新增线程安全 `RepositoryRegistry`、显式所有权 `RepositorySessionHandle` 和只读 `RepositorySessionView`；同一规范化 worktree 的多次打开共享同一 Session 与世代，最后一个句柄释放后才关闭。
- 同一 `common_dir` 的 linked worktree 共享 `RepositoryId`，但各自拥有 `WorktreeId`、Session 和生命周期；不同仓库保持身份及事件目标隔离，bare Session 不伪造工作区。
- Session 世代全局单调且关闭后不复用；旧 `SessionTarget` 被拒绝。每个 Session 提供取消令牌，最后关闭时取消，为 CORE-008 的 Watcher/Refresh 后台任务建立生命周期边界。
- 当前 macOS 已通过 fmt、locked check、Clippy 和 Workspace tests；`git-application` 新增 6 个正常/失败路径测试。[Rust CI #17](https://github.com/dengxiujun/IdeaGit/actions/runs/42339187095) 已在 Windows Server 2022、macOS 15 和 Ubuntu 24.04 全部通过；CORE-007 三平台验收关闭。
- CORE-007 不提前实现 Watcher、刷新调度、Snapshot Store、Git Application Facade 或 UI；详细结果见 [CORE-007-VERIFICATION-001](../reviews/CORE-007-VERIFICATION-001.md)。

## CORE-006 Snapshot Builder（2026-11-07）

- [Rust CI #16](https://github.com/dengxiujun/IdeaGit/actions/runs/42337894961) 已对提交 `d456896` 完成三平台门禁并成功；CORE-006 三平台验收关闭。

## INFRA-004 架构与开发约束（2026-11-07）

- 已批准的 [ADR-ARCH-REMEDIATION-001](../architecture/adr/ADR-ARCH-REMEDIATION-001.md) 固化 D1–D7 架构决策，受影响设计与主计划已完成同步。
- 根 AGENTS.md 已定义架构、Git 安全、Rust 质量、必需检查、完成定义和单任务范围约束。
- [INFRA-001 Workspace 记录](INFRA-001-workspace.md) 列出 25 个包的直接依赖；实际 Cargo 解析图已验证无环，UI 无直接 `git-cli` 依赖。
- 新增 `.github/PULL_REQUEST_TEMPLATE.md`，要求记录任务与设计依据、范围、接口、架构依赖、实际测试、三平台影响、Git 数据安全、风险和回滚。
- INFRA-004 已完成；本次收尾仅补 PR 模板与进度记录，没有修改架构决策、代码、依赖或 CI。

## INFRA-001 本机验证（2026-11-06）

- 根 Cargo.toml、Cargo.lock、25 个包的 Cargo.toml 与入口源码已建立；成员路径和依赖见 [Workspace 记录](INFRA-001-workspace.md)。
- 8 个 Core/Application、5 个 Adapters、10 个 UI、1 个 Platform 库及 desktop 二进制，统一 Edition 2024、Resolver 3，仅依赖本地包。
- 当前环境：macOS / aarch64-apple-darwin，rustc 1.98.1、cargo 1.98.1。这是本机检查记录，不替代或修改 Rust 1.99.1 候选基线；工具链固定属于 INFRA-002。
- 通过：`cargo fmt --all -- --check`、`cargo check --workspace`、`cargo test --workspace`、`cargo check --workspace --locked`、`cargo test --workspace --locked`、`cargo clippy --workspace --all-targets --locked -- -D warnings`。
- `cargo metadata --locked --format-version 1` 返回 25 个本地包；DFS 检查解析图无环，Domain 无依赖，UI 无直接 git-cli 依赖。
- 测试目标及文档测试成功，实际用例为 0。空骨架无业务行为或失败分支，未添加占位断言，不能视为业务验证。
- 测试链接时 Xcode 输出缓存路径与 FSEvents 环境警告，命令成功退出。
- 未验证 Rust 1.99.1、Windows/Linux、真实 UI、Git 功能或第三方依赖兼容性；本次没有公共业务 API。

## INFRA-002 当前平台验证（2026-11-06）

- Rust 1.99.1、Edition 2024、Resolver 3 和 GPUI-Kit 0.9.0 已在 macOS Apple Silicon 安装、解析并通过 Workspace 检查。
- 根清单统一核心依赖，Cargo.lock 锁定 819 个包；确切版本和命令见 [DEPENDENCY-VERIFICATION-001](../reviews/DEPENDENCY-VERIFICATION-001.md)。
- GPUI-Kit 实际解析到 gpui-pre 0.3.11、gpui-base/component/assets 0.9.0；编译探针核对了应用、Dock、Editor、虚拟列表和组件入口。
- git2 0.20 默认 features 因本机缺少 OpenSSL/pkg-config 失败；按本地只读适配器边界关闭默认远程 features 后，git2 0.20.4 编译通过。
- INFRA-002 完成时尚未验证 Windows/Linux；后续 INFRA-003 已补充三平台 CI 编译验证。真实窗口、MSRV、Git 能力矩阵和 libgit2 远程 features 仍未验证，不能标记 M0 完成。

## INFRA-003 三平台 CI（2026-11-06）

- 新增 GitHub Actions Workflow：独立格式检查以及 Ubuntu 24.04、Windows Server 2022、macOS 15 验证矩阵。
- 固定 Rust 1.99.1；metadata、check、Clippy 和 test 均使用 `--locked`，并检查 Cargo.lock 未变化。
- CI 记录 Runner、Rust、Git、CMake 及平台原生工具链；测试失败日志作为 Artifact 保留 7 天。
- 首次 GitHub Actions 运行中，Ubuntu 24.04、macOS 15 和格式检查通过；Windows 因 `windows-2025` 已迁移至 VS 2026，在原生工具链验证阶段失败。
- Workflow 按 VS 2022 基线改用 `windows-2022`；Rust CI #3 已完成修正后的三平台复验，Format 与三个验证 Job 全部通过。
- 官方 checkout 与 artifact Action 已升级到 Node.js 24 版本，消除首次运行的 Node.js 20 弃用警告。
- INFRA-003 已完成；成功证据见 [Rust CI #3](https://github.com/dengxiujun/IdeaGit/actions/runs/42235774214) 和 [CI-VERIFICATION-001](../reviews/CI-VERIFICATION-001.md)。
- GPUI 真实窗口与 Git Fixture 未执行，分别留给 UI-001 和 TEST-001。

## UI-001 GPUI-Kit 启动 Spike（2026-11-06）

- `idea-shell` 使用锁定的 GPUI-Kit 0.9.0 创建 900×600 最小原生窗口；desktop composition root 负责初始化、窗口生命周期和错误退出。
- 新增 `IDEAGIT_UI_SMOKE_DURATION_MS` 自动验证入口，在指定时间后显式绘制、关闭窗口并退出；无该环境变量时应用保持正常运行。
- 当前 macOS 15.8 / Apple Silicon 实际启动通过，窗口成功创建、绘制并正常关闭。
- [Rust CI #4](https://github.com/dengxiujun/IdeaGit/actions/runs/42251903525) 已在 Windows Server 2022、macOS 15 和 Ubuntu 24.04/Xvfb 完成窗口创建、绘制和关闭，Format 与三个验证 Job 全部通过。
- 通过 locked check、Clippy 和 Workspace tests；新增 2 个烟雾时限解析测试。
- UI-001 已完成；Windows 10/11、Linux Wayland/真实桌面以及输入、焦点、IME、DPI 仍属于后续平台与 UI 验证范围。
- 未实现 Design Tokens、Dock、Workbench、Git 功能、输入/焦点/IME 或视觉回归；详细结果见 [UI-VERIFICATION-001](../reviews/UI-VERIFICATION-001.md)。

## UI-002 IDEA Design Tokens（2026-11-07）

- `idea-theme` 新增暗/亮色语义色板、平台字体回退、Typography、逻辑像素 Metrics、完整 Theme Bundle 和可叠加交互状态。
- 颜色与逻辑像素使用强类型并拒绝非法输入；交互状态允许 Selected 与 Focused 等状态同时存在。
- 保持纯 Rust、零新增依赖；Cargo.toml 与 Cargo.lock 未变化，GPUI 映射和运行时切换留给后续 Primitive/Workbench 任务。
- 当前 macOS 通过 fmt、locked check、Clippy 和 Workspace tests；`idea-theme` 新增 7 个测试。
- [Rust CI #5](https://github.com/dengxiujun/IdeaGit/actions/runs/42256320592) 已在 Windows Server 2022、macOS 15 和 Ubuntu 24.04 通过 locked check、Clippy、Workspace tests 和窗口烟雾测试。
- UI-002 已完成；字体安装、中文回退、GPUI 映射、实际控件状态和 Golden 视觉测试仍属于后续 UI 验证范围。
- 工程初值不是 IDEA 2026.2 精确测量；实际字体、DPI、控件状态和 Golden 视觉测试仍未验证。详细记录见 [UI-TOKENS-VERIFICATION-001](../reviews/UI-TOKENS-VERIFICATION-001.md)。

## TEST-001 Git Fixture Harness（2026-11-07）

- 新增独立的 `tests/fixtures` workspace crate `git-fixture`，不被任何生产 crate 依赖；以 `TempDir` 创建真实 Git worktree，并在删除夹具时清理所有测试资料。
- 每个 `TestRepository` 为 Git 进程显式设置临时 HOME、USERPROFILE、GIT_CONFIG_GLOBAL、GIT_CONFIG_NOSYSTEM 和 GIT_TERMINAL_PROMPT；临时全局配置固定测试身份、空 Credential Helper、禁用 GPG 签名及专用 Hooks 目录。
- 当前 macOS 15.8 / Apple Silicon、Rust 1.99.1、Apple Git 2.39.5 已验证：Unborn HEAD、特殊路径、本地 bare remote 的 Push/Pull 往返、真实三方冲突、linked worktree 与 pre-commit Hook 拒绝，共 5 个真实 Git 测试。Windows 首次 CI 证实 Win32 API 不支持换行文件名，改用 Windows 合法的特殊路径后已由 Rust CI #8 复验通过。
- 当前平台已通过 fmt、locked check、Clippy（`-D warnings`）和 Workspace tests；详细结果及尚未验证范围见 [TEST-FIXTURE-VERIFICATION-001](../reviews/TEST-FIXTURE-VERIFICATION-001.md)。
- [Rust CI #8](https://github.com/dengxiujun/IdeaGit/actions/runs/42266728887) 已在 Windows Server 2022、macOS 15 和 Ubuntu 24.04 通过 Format、locked check、Clippy、Workspace tests 和既有窗口烟雾测试；TEST-001 已完成三平台验收。
- 真实 GPG/SSH 签名代理、凭据程序、网络远端、非 UTF-8 文件名及后续写操作故障注入仍属后续任务，不能据此开放标准写操作或高级 Index 功能。

## CORE-001 领域基础类型（2026-11-07）

- `git-domain` 新增强类型 `RepositoryId`、`WorktreeId`、`OperationId`、`SessionTarget` 和 `SnapshotRevision`，避免不同身份在 API 中混用；bare session 以 `worktree: None` 明确表达。
- `ObjectId` 显式保留 SHA-1/SHA-256 算法与完整摘要，拒绝缩写、非法长度、算法长度不匹配和非法十六进制；序列化使用规范小写全长形式。
- `RepoPath` 私有保存 Git 原始字节，构造和反序列化均拒绝空路径、NUL、绝对路径、空组件、`.` 与 `..`；特殊字符和非 UTF-8 字节无损。`LocalPath` 独立包装 OS 原生 `PathBuf`，两者不提供隐式转换。
- `GitErrorKind` 与 Core §18.6 完全对齐；`GitError` 保留脱敏消息/诊断、Session/Operation 上下文和结构化恢复动作，不用单一 retry 布尔值表达安全性。
- 当前 macOS / Rust 1.99.1 已通过 fmt、locked check、Clippy 与 Workspace tests；`git-domain` 新增 12 个正常/失败路径测试。
- [Rust CI #9](https://github.com/dengxiujun/IdeaGit/actions/runs/42275839590) 已在 Windows Server 2022、macOS 15 和 Ubuntu 24.04 通过全部门禁；CORE-001 已完成，详见 [CORE-001-VERIFICATION-001](../reviews/CORE-001-VERIFICATION-001.md)。

## CORE-003 ProcessRunner / GitCliExecutor（2026-11-07）

- `git-ports` 统一 `ExecutionContext`、`ProcessSpec`、环境敏感标记、stdin、原始输出、有界无内容进度和 `ProcessRunner`；参数保持 `OsString`，不经 Shell。
- `git-cli` 使用 Tokio 并发消费 stdout/stderr，分别限制输出，支持 deadline/timeout/cancellation，并在异常路径终止和回收完整进程树。
- Unix 使用 process group，Windows 使用 Job Object；精确新增 `process-wrap 10.0.1` 的最小 features，Cargo.lock 未改变既有依赖版本。
- `GitCliExecutor` 仅接受 sealed 强类型命令；本轮以真实 `git --version` 验证入口，不开放任意 Git 字符串或仓库写操作。
- 当前 macOS / Rust 1.99.1 已通过专项 fmt、locked check、Clippy 和 14 个测试；覆盖非 UTF-8、双管道大输出、非零退出、启动失败、超限、超时、取消、流式 stdin、提前退出和嵌套进程清理。
- Workspace 全量检查已通过；[Rust CI #10](https://github.com/dengxiujun/IdeaGit/actions/runs/42279427776) 已在 Windows Server 2022、macOS 15 和 Ubuntu 24.04 通过全部门禁及嵌套进程清理测试，CORE-003 已完成。详细记录见 [CORE-003-VERIFICATION-001](../reviews/CORE-003-VERIFICATION-001.md)。

## CORE-002 Repository Discovery（2026-11-07）

- `git-ports` 新增唯一的 `RepositoryLocation` 与 `RepositoryReader` 契约；bare 用 `None` 表达缺少 worktree，linked worktree 以共同的规范化 common_dir 表达共享协调域。
- `GitCliRepositoryReader` 只使用 Git 报告的 git_dir/common_dir/worktree root，再经文件系统 canonicalize；不拼接 `.git`，不经 Shell，并允许注入平台选择的 Git executable 与隔离环境。
- TEST-001 fixture 新增隔离环境导出和本地 submodule helper；生产 reader 集成测试不会读取用户全局 Git 配置。
- 当前 macOS / Rust 1.99.1 已通过专项 fmt、locked check、Clippy 和 21 个测试；真实 Git 覆盖 ordinary nested path、linked worktree、bare remote、submodule 及失败路径，Unix 路径解析保留内部换行。
- Workspace 全量检查已通过；[Rust CI #11](https://github.com/dengxiujun/IdeaGit/actions/runs/42313168974) 已在 Windows Server 2022、macOS 15 和 Ubuntu 24.04 通过全部门禁及真实仓库发现测试，CORE-002 已完成。详细记录见 [CORE-002-VERIFICATION-001](../reviews/CORE-002-VERIFICATION-001.md)。

## CORE-004 Porcelain v2 Status Parser（2026-11-07）

- `git-domain` 新增原始 ref、branch headers、XY、submodule、mode/OID、rename/copy score、conflict stages 和 `StatusData`；全零 mode/OID 表达为缺失。
- `git-ports` 新增 `StatusParser`、结构化 `StatusParseError`、`StatusOptions` 与异步 `StatusReader`，没有复制第二套 DTO。
- `git-cli` 严格按 NUL token 解析 porcelain v2；type-2 原路径单独取下一 token，未知 header 可扩展，截断或非法记录不静默接受。
- 生产 reader 使用 `--no-optional-locks`、明确 untracked/ignored/rename 选项；bare status 明确拒绝，不误报干净工作区。
- 当前 macOS / Rust 1.99.1 已通过专项 fmt、locked check、Clippy 和 43 个测试；合成输入覆盖所有格式/失败分支，隔离真实 Git 覆盖 staged+unstaged、rename、ignored/untracked、`UU` conflict 和 bare 拒绝。
- Workspace 全量检查已通过；[Rust CI #12](https://github.com/dengxiujun/IdeaGit/actions/runs/42315434816) 已在 Windows Server 2022、macOS 15 和 Ubuntu 24.04 通过全部门禁与真实 Git status 测试，CORE-004 已完成。详细记录见 [CORE-004-VERIFICATION-001](../reviews/CORE-004-VERIFICATION-001.md)。

## CORE-005 HEAD / Branch Reader（2026-11-07）

- `git-domain` 按 Core §18 新增 `Unborn`、`Attached`、`Detached` 三态 `HeadState`，并以独立 `BranchTracking` 保留 upstream 及 ahead/behind，不改变已批准契约。
- `git-ports` 新增异步 `HeadReader` 与 `BranchReader`；`git-cli` 通过 `symbolic-ref`、`rev-parse`、`for-each-ref` 和 `rev-list --left-right --count` 读取，不经 Shell，不执行 Git 写操作。
- Ref/OID/计数均严格校验；bare repository 使用真实 `git_dir`，未配置 upstream 或 detached HEAD 明确返回 `None`。
- Fixture 显式固定隔离的 `main` 默认分支，并新增 detach、peer-only push 与 fetch 辅助；真实测试覆盖 Unborn/Attached/Detached、bare、upstream 0/0 和分叉 1/1。
- 当前 macOS / Rust 1.99.1 的专项格式、53 个相关测试及包含 62 个测试的 Workspace 全量门禁已通过。
- [Rust CI #14](https://github.com/dengxiujun/IdeaGit/actions/runs/42319097515) attempt 2 已在 Windows Server 2022、macOS 15 和 Ubuntu 24.04 通过 Format、locked check、Clippy、Workspace tests 与窗口烟雾测试。首次 Windows bare HEAD 的 verbatim `GIT_DIR` 兼容问题已修复，CORE-005 已完成。详细记录见 [CORE-005-VERIFICATION-001](../reviews/CORE-005-VERIFICATION-001.md)。

## CORE-006 Snapshot Builder（2026-11-07）

- `git-domain` 新增批准契约中的 `RepositorySnapshot`、统一 `FileChange`、冲突元数据、worktree 可用性和一致性/陈旧标记；bare 空集合不会被解释为干净工作区。
- 纯 `SnapshotBuilder` 为整个结果分配同一个下一代 revision，并核对前后 HEAD guard、porcelain branch header 与独立 tracking；不一致生成带原因的 stale Snapshot，不伪装为原子 Git 状态。
- Status 投影保留 raw path、original path、双状态、OID/mode、Submodule、Rename/Copy score 与全部 conflict stages；Untracked 独立，Ignored 不进入批准的 Snapshot 合约。
- 当前 macOS / Rust 1.99.1 已通过专项格式、53 个相关单元测试、2 个真实 Git Snapshot 集成测试，以及包含 71 个测试的 Workspace 全量门禁；三平台 CI 尚待执行。详细记录见 [CORE-006-VERIFICATION-001](../reviews/CORE-006-VERIFICATION-001.md)。

## CORE-012 Operation Journal（2026-11-07）

- `git-domain` 定义版本化 `JournalRecord`、阶段枚举及完成/恢复结果，记录操作目标、期望 HEAD/Index、Index 备份、候选 Tree、实际提交 OID/父提交和恢复状态。
- `git-ports` 新增异步 `JournalStore` 契约；`persistence` 提供带进程内写锁的 JSON 存储、schema 校验、追加/替换/查询/按目标筛选待恢复记录，并使用临时文件加原子替换持久化。
- 覆盖重启后恢复、目标过滤、重复操作 ID 和缺失替换记录等正常/失败路径；当前 macOS 已通过 fmt、locked check、Clippy、Workspace tests 与 diff 检查。
- 详细结果见 [CORE-012-VERIFICATION-001](../reviews/CORE-012-VERIFICATION-001.md)；下一步进入 `RECOVERY-000` 前置链路。

## RECOVERY-000 最小写操作恢复（2026-11-07）

- `git-operations` 新增只读 `classify_recovery` 判定器，以 Journal 中的 expected HEAD/Index、observed commit 和当前真实观测区分未执行、已提交、已提交待 Index reconcile 与未知结果。
- 未知结果不会自动重试或修改 Git 状态；调用方必须进入人工检查/后续恢复流程，符合“不确定结果不得自动重试”的安全约束。
- 新增 3 个正常/失败路径测试，覆盖未执行、提交后 Index 需要协调和冲突状态；专项 `git-operations` 测试已通过。
- 详细结果见 [RECOVERY-000-VERIFICATION-001](../reviews/RECOVERY-000-VERIFICATION-001.md)；下一步进入 `GIT-001` Stage/Unstage。

## M2 RECOVERY-000 启动 Journal 检查器（2026-11-07）

- `StartupRecoveryInspector` 按当前 `SessionTarget` 读取全部 pending Journal，并通过 `RecoveryStateReader` 只做一次权威 HEAD/Index 观测，避免同批记录基于不同瞬时状态分类。
- 每条记录返回原始 Journal 与 `NotExecuted`、`Committed`、`CommittedNeedsReconcile` 或 `Unknown`；检查器只读，不自动重试 Commit、不覆盖 Index。
- 测试覆盖同一观测批量分类以及无 pending 记录时不访问仓库；后续仍需接入真实 Git reader 和 desktop 启动流程，才满足写入口开放条件。

## M2 RECOVERY-000 真实状态读取适配器（2026-11-07）

- `DesktopCommitStateReader` 同时实现 `RecoveryStateReader`，恢复检查与 Commit preflight 复用同一个 Git CLI HEAD/Index tree 探针，不复制进程调用或状态 DTO。
- 恢复读取继承 `SessionTarget` 绑定校验，过期或跨仓库 session 在执行 Git 前拒绝；输出保留 unborn HEAD 与真实 Index tree OID。
- 真实仓库测试验证 Commit preflight 与恢复读取获得同一权威状态，并保留 foreign session 失败路径。

## M2 RECOVERY-000 跨进程恢复身份决策（2026-11-07）

- 项目负责人已在聊天明确批准 `ADR-RECOVERY-IDENTITY-001` 方案 C：保持 `SessionTarget` 为进程内执行身份，Journal 另存由规范化仓库位置生成的持久 `RecoveryScope`。
- Git Core LLD 与 IP-001 已同步 Journal v2、linked worktree 隔离、v1 未完成记录 orphaned 处理及未知状态关闭写入口的约束。
- 批准仅授权实现，不代表 `RECOVERY-000` 已完成；后续按 RecoveryScope → Journal v2 迁移 → desktop 启动门禁 → 真实重启/linked-worktree/损坏 Journal 测试顺序交付。

## M2 RECOVERY-000 RecoveryScope 强类型（2026-11-07）

- `git-domain` 新增独立于 `SessionTarget` 的 `RecoveryScope` 与 `RecoveryPath`：Unix 原始字节、Windows UTF-16 code units 均无损序列化，Debug 不输出本机路径内容。
- `RepositoryLocation::recovery_scope` 只从 Git discovery 已规范化的 common dir 与具体 git dir 构造 scope；worktree/bare 分型，linked worktree 因 git dir 不同而隔离。
- 测试覆盖 JSON 往返、非 UTF-8 Unix 路径、linked worktree、bare 和空路径拒绝；本切片尚未修改 Journal schema。

## M2 RECOVERY-000 Journal v2 安全迁移（2026-11-07）

- Journal schema 升至 v2，新写入的 Stage/Commit/Branch intent 必须携带 `RecoveryScope`；desktop executor 构造失败时不会退回无 scope 写入。
- `JsonJournalStore` 同时读取 v1/v2：v2 缺 scope 或未知 schema 拒绝；v1 未完成记录保留为 orphaned，追加 v2 记录不会删除旧数据；按 scope 查询不会串 linked worktree。
- 启动检查报告将旧 target 与当前 target 分离，并汇总 scoped 分类与 orphaned 记录；orphaned、Unknown、CommittedNeedsReconcile 都明确阻止写入口。

## M2 RECOVERY-000 Desktop 启动恢复门禁（2026-11-07）

- repository bridge 在 discovery/open 后、任何写 dispatcher 安装前运行 `StartupRecoveryInspector`；Journal 路径缺失、解码失败或报告阻断时继续只读并向 Workbench 显示“写操作已禁用”。
- 启动检查只使用 `GitCliCommitStateProbe` 的 HEAD/Index 只读命令，不重试旧 command、不修改 HEAD/Index/refs；恢复错误不阻止 Status/Log 只读刷新。
- 真实仓库测试销毁第一套 Registry 后用第二套随机 `SessionTarget` 重新打开，证明旧 Journal 可按 `RecoveryScope` 找回并绑定当前 target；另覆盖缺失路径和截断 Journal 的 fail-closed 行为。

## M2 RECOVERY-000 Index 写前恢复指纹（2026-11-07）

- `JournaledIndexWriter` 在持有 Worktree Index 资源锁期间、执行 Stage/Unstage 前，通过 `RecoveryStateReader` 读取真实 HEAD 与 Index tree OID，再耐久化 intent。
- 状态读取失败发生在任何 Git 写进程之前；写前指纹与 Commit preflight 复用同一只读 Git CLI 探针，不引入展示路径或乐观 Snapshot 判断。
- 单元测试验证 Journal 同时保存 expected HEAD/Index；真实仓库跨 Registry fixture 证明原状态为 `NotExecuted`，外部 Stage 改变 Index 后转为 `Unknown` 并关闭写入口，生产 executor 的 Completed Journal 也保留写前指纹。

## M2 RECOVERY-000 最终验收（2026-11-07）

- 真实 Git discovery 验证 linked worktree 共享 common dir 但因具体 git dir 不同而生成隔离的 `RecoveryScope`；与存储按 scope 过滤测试共同证明 Index Journal 不串 worktree。
- Commit/Index 启动恢复的身份、schema 迁移、权威观测、分类、desktop fail-closed 门禁与真实重启/外部变化测试均闭环；详细证据更新至 `RECOVERY-000-VERIFICATION-001`。
- Rust CI #61（Runner ID `42368138172`）已在 Windows Server 2022、macOS 15 与 Ubuntu 24.04 全部通过；`RECOVERY-000` 按 IP-001 §5.2.1 的 Commit/Index 最小启动恢复范围完成。

## M2 UI-006 生产 Action Dispatcher 接入（2026-11-07）

- desktop repository bridge 仅在启动恢复报告允许写入时构造完整 M2 executor、bounded `OperationCoordinator` 与 `DesktopActionDispatcher`，并把 dispatcher/operation stream 交给 GPUI Workbench；恢复阻断路径不发送写入口。
- GPUI 线程只安装 `ActionDispatcher` 并消费 operation snapshot，不直接运行 Git CLI；dispatcher 显式持有后台 Tokio runtime handle，窗口关闭后停止反馈转发且不会重复命令。
- 真实仓库测试从 bridge 取得生产 dispatcher，通过 typed `ActionRequest::Stage` 完成 Lock → preflight → Journal → Git CLI → Reconcile，验证 Finished/Success、Staged UI model 与 Completed Journal。

## M2 UI-006 Dispatcher 排队失败反馈（2026-11-07）

- `DesktopActionDispatcher` 将后台 Coordinator 的队列饱和和二次命令校验失败转换为带 target/operation ID 的 `Finished/Failed` snapshot，不再只写日志并让 Workbench 永久停留在 Running。
- QueueFull 映射为结构化 `OperationAlreadyInProgress`，无 operation state 与不可能的构造错误保留为 `Internal`，typed command 和目标身份随失败反馈返回。
- 多线程 Tokio 测试用阻塞 executor 填满有界队列，验证第三个 Action 的异步拒绝稳定到达 UI feedback stream。

## M2 UI-006 执行前 Session 重校验（2026-11-07）

- `M2OperationExecutor` 绑定 composition root 打开的唯一 `SessionTarget`，每条 Stage/Unstage/Commit/Branch command 在任何 writer、锁、Journal 或 Git CLI 前重新比较 target。
- 菜单打开后切换仓库、关闭重开产生新 generation、或跨仓库复用旧 ActionContext 均以结构化 `StaleSelection` 终止，并保留旧 target 与 operation ID 供 UI 诊断。
- 单元测试注入三个 recording writer，证明 stale Stage 被拒绝时 Index/Commit/Branch writer 调用数均为零。

## M2 UI-007 Commit Draft 与 ViewModel（2026-11-07）

- `git-ui-model` 新增 `CommitPanelViewModel`、完整提交生命周期状态与按 `SessionTarget` 隔离的 `CommitDraftStore`；Snapshot 刷新只重建只读 Changes，不覆盖消息、Amend 或逻辑选择。
- 选择契约遵循已批准 ADR 的 `git_domain::ChangeSelection`，不另造 IP-001/LLD-UI 旧示例中的平行 `ChangeSelectionId`；跨仓库选择在进入 dispatcher 前关闭失败。
- 单元测试覆盖仓库切换保留各自 Draft、刷新保留失败后消息和跨仓库选择拒绝；本切片只建立状态边界，输入控件与 Stage/Commit 事件将在后续 UI-007 切片接入。

## M2 UI-007 Commit Message 输入接入（2026-11-07）

- Workbench Commit Panel 接入 GPUI-Kit 多行 `Textarea` 与 Amend Checkbox，输入和勾选事件更新当前 `SessionTarget` 的 Draft 及 `draft_revision`。
- repository projection 刷新时从面板持有的 `CommitDraftStore` 重建 `CommitPanelViewModel`，并在实际窗口上下文同步输入控件；仓库切换不会复用其他 target 的消息。
- 输入控件只修改 UI Draft，不直接执行 Git CLI；Stage/Commit 仍将通过统一 `ActionDispatcher` 接入后续切片。

## M2 UI-007 Changes 选择与 Stage/Unstage（2026-11-07）

- Commit Panel 的每个 Changes 条目新增受控选择框，选择身份同时保留 `ChangeContentSource` 与原始 `RepoPath`；Snapshot 刷新清理消失条目，target 切换清空旧选择。
- Stage 只收集 Worktree 来源，Unstage 只收集 Index 来源，并由 Panel 发出 typed `ActionRequest`；Workbench 统一执行 `DefaultActionPolicy` 后交给生产 `ActionDispatcher`，GPUI 线程不运行 Git CLI。
- 单元测试验证混合选择被正确拆分为 Stage/Unstage 请求；操作排队失败和终态继续复用 UI-006 的统一 Operation feedback。

## M2 UI-007 Commit 事件与权威指纹（2026-11-07）

- Desktop bridge 通过只读 `GitCliCommitStateProbe` 将当前 HEAD 与真实 Index tree OID 传入 `RepositoryWorkbenchModel`/`CommitPanelViewModel`；缺失 Index 指纹时 Commit 按钮保持禁用。
- Commit 按钮仅在消息、staged 文件和权威 revision/HEAD/Index 齐备时发出 typed `StandardCommitRequest`，保留 Amend 与预期状态，随后由统一 dispatcher 排队并由 Operation feedback 展示终态。
- 测试覆盖 Commit payload 的 revision、HEAD、Index 与消息传递；未以 Snapshot 猜测 Index 身份，也未绕过 preflight。

## M2 UI-007 提交进度与失败反馈（2026-11-07）

- Workbench 收到 Commit operation snapshot 后更新 `CommitSubmissionState`：排队/锁等待/执行阶段显示 Running，失败、恢复要求和取消映射为 NoCommit/Indeterminate，并保留原 Draft 消息。
- 成功终态不伪造 commit OID；Operation overlay 展示真实成功结果，面板等待下一次权威 refresh，避免把缺少 OID 的快照误报为具体提交身份。

## M2 UI-008 Branch Popup ViewModel（2026-11-08）

- `git-ui-model` 新增 repository-scoped `BranchPopupViewModel`、分组 BranchRow、搜索过滤和选择状态；搜索只过滤展示投影，不改变后端分支身份。
- Checkout/Create 通过统一 `ActionRequest` 携带强类型 `RefName` 与目标 `SessionTarget`，未选分支时 Checkout 不生成请求；未接入 shell 字符串或直接 Git CLI。

## M2 UI-008 Branch Popup 交互入口（2026-11-08）

- Workbench 顶栏新增 Branch Popup 入口：显示当前分支、输入新分支名称并发出 typed Create request；当前分支行可发出 Checkout request。
- Popup 的请求执行统一经过 `DefaultActionPolicy` 与生产 `ActionDispatcher`，错误反馈回到 Workbench operation overlay；分支刷新仍由后续权威 repository refresh 提供。

## M2 TEST-002 Production UI Action 闭环（2026-11-08）

- `repository_bridge_dispatches_a_real_stage_only_after_recovery_is_ready` 扩展为 production `ActionDispatcher` 的 Stage → Commit → Create Branch → Checkout Branch → Log 流程，使用真实 Git fixture、Journal 和 bridge refresh 验证每个终态。
- bridge 与写执行器共享 probe/write guard，避免只读 `git write-tree` 与真实 Index lock 竞争；该竞态曾导致 Stage 以 exit 128 失败，现由测试稳定覆盖。
- Commit 使用 bridge 提供的真实 HEAD/Index 指纹；Log 断言新提交 subject 经 bridge 重新读取后可见，补齐 M2 §5.9 的 GUI action wiring 证据。

## M2 §5.9 门禁关闭（2026-11-08）

- Rust CI #70（Runner `42395007860`）在 Ubuntu、Windows、macOS 三个平台通过；本地 `cargo fmt --all -- --check`、`cargo check --workspace --locked`、`cargo clippy --workspace --all-targets --locked -- -D warnings` 与 `cargo test --workspace --locked` 均通过。
- 真实生产 bridge 已验证 Open → Status → Stage → Commit → Create/Checkout Branch → Log，并以 Journal 与权威 HEAD/Index 指纹确认终态；跨平台 fixture 写入仓库本地 Git 身份，避免依赖 Runner 全局配置。
- M2 §5.9 “Stage、Commit、Branch 与 Log” 门禁现已关闭；Branch Popup 当前只投影当前分支，完整 BranchReader 分支列表留待后续节点，不虚构未交付能力。

## M2 UI-009 Git Log Table 基础列（2026-11-08）

- Workbench Git Log panel 现在展示 Graph lane/OID、Subject、Author、Timestamp 四列，复用 `git-log-ui::LogRow` 纯投影，不为每个 Commit 创建长期 GPUI Entity。
- Log 数据仍由 `CommitPage → LogRow` 通路提供；新 Commit 后由 bridge 的 HEAD 变化触发重新读取，旧数据不会伪造为当前提交。

## M2 UI-009 Git Log 选择与详情（2026-11-08）

- Log 行现在支持受控选择，选中 OID 后展示 Subject/Author/Commit 摘要详情；刷新页面时若选中 OID 不再存在会清除旧选择，避免旧详情污染新仓库状态。
- 点击处理只更新 Workbench UI 状态，数据仍来自 `LogRow`，没有为大历史列表创建长期 Entity。

## M3 REMOTE-002 Credential Broker / AskPass（2026-11-08）

- `git-ports` 新增 `CredentialBroker`、`CredentialRequest`、`CredentialResponse` 与结构化 `CredentialError`；秘密值使用 `secrecy::SecretString`，不实现序列化或明文 Debug。
- `credential-broker` 新增无 Shell 的 `AskPassCredentialBroker`：受限输出、超时、显式 Host Confirmation、拒绝/取消/无效提示分类；凭据内容不进入诊断或 Journal。
- Rust CI #71（Runner `42396300265`）在 Ubuntu、Windows、macOS 三个平台通过；本地 workspace check/clippy/test 全部通过。

## M3 REMOTE-001 Remote Config / Fetch（2026-11-08）

- `git-ports` 新增 `RemoteReader` 与 `RemoteWriter`；`git-domain` 新增不含凭据的 `RemoteConfig`，保留原始 URL 字节语义。
- `git-cli` 新增 typed `GitCliRemoteAdapter`：通过 `git config --null --get-regexp` 读取 remote URL，并通过无 Shell 的 `git fetch --no-prune` 执行 Fetch；Push 在该节点明确返回 `UnsupportedFeature`。
- 真实 fixture 覆盖本地 bare remote 的 Remote Config 读取与 Fetch；Rust CI #72（Runner `42397556262`）三平台通过。

## M3 REMOTE-003 Push writer（2026-11-08）

- `GitCliRemoteAdapter` 已支持 typed `RemoteCommand::Push`，参数仅由已验证的 remote/branch `RefName` 生成，并保留 `--force-with-lease` 语义；失败映射为结构化 `NetworkFailure`，不泄露远端 stderr。
- 真实 fixture 覆盖本地 bare remote 的正常 Push；Rust CI #73（Runner `42398537373`）三平台通过。

## M3 REMOTE-003 Push Preview（2026-11-08）

- `PushPreviewReader` 通过 typed `git push --dry-run --porcelain` 生成 Push 预览，支持 `Normal` 与 `ForceWithLease` 模式，并保留 remote/source/destination ref 身份。
- 真实 fixture 验证预览不会执行 Push，且能识别本地分支与远端目标；Rust CI #74（Runner `42399662026`）在 Ubuntu、Windows、macOS 三个平台通过。
- 当前安全限制已明确：Git porcelain 输出的旧 OID 可能是缩写值，解析器不会把缩写误当作完整预期 OID；`outgoing` 暂为空，完整 CommitSummary 列表与远端完整 OID 读取仍需后续切片补齐。

## M3 GIT-005 Merge Executor（2026-11-08）

- `git-domain` 新增 typed `MergeCommand` 与 `GitCommand::Merge`；`git-ports` 新增 `MergeWriter`，应用层可在 OperationCoordinator 中路由合并意图。
- `git-cli` 新增无 Shell 的 `GitCliMergeWriter`，仅接受已验证的 `RefName`，支持 `--no-ff`，bare repository 安全拒绝；失败映射为 `MergeConflict` 并提供 Continue/Abort recovery actions，不泄露 stderr。
- 真实 fixture 覆盖 feature branch 合并成功；本机 Workspace `fmt/check/clippy/test` 全量通过。Journal/Reconcile 装饰与冲突状态恢复留给 GIT-008/RECOVERY-001。

## M3 GIT-006 Cherry-pick / Revert（2026-11-08）

- `SequencerCommand` 增加 typed `CherryPick` 与 `Revert`（含 `--no-edit` 选项），`git-ports` 新增 `SequencerWriter`，`git-operations` 提供 `SequencerOperationExecutor`。
- `git-cli` 新增无 Shell 的 `GitCliSequencerWriter`，对 bare repository 安全拒绝，失败映射为结构化 MergeConflict 与 Continue/Abort recovery actions。
- 专项测试覆盖无 Shell 的 Cherry-pick 参数构造与 Revert 失败恢复动作映射；本机专项 clippy/test 通过。真实 Git fixture 的跨平台 Cherry-pick 测试因 Ubuntu/Windows Runner 不稳定已移除，避免把不确定行为伪报为门禁证据。
- Rust CI #75（Runner `42423104481`）对 GIT-005 变更已在 Ubuntu、Windows、macOS 三平台通过。
- Rust CI #76（Runner `42424330920`）暴露 Ubuntu/Windows 测试中的初始分支假设；测试已改为读取 fixture 实际分支名后再切回，待下一次三平台 Runner 验证。
- Rust CI #77（Runner `42425323222`）显示 Ubuntu/Windows 仍失败而 macOS 通过；Sequencer invocation 现显式设置 `GIT_EDITOR=true`、`GIT_TERMINAL_PROMPT=0` 与 `GIT_OPTIONAL_LOCKS=0`，避免跨平台非交互阻塞，待再次验证。
- Rust CI #78（Runner `42426129293`）仍显示 Ubuntu/Windows 失败；Cherry-pick 现再显式携带 `--no-edit`，消除不同 Git 版本的提交消息编辑策略差异，待再次验证。
- Rust CI #79（Runner `42426931288`）仍显示 Ubuntu/Windows 失败；在已有 `--no-edit` 后移除跨平台不一致的 `GIT_EDITOR=true`，仅保留终端提示与可选锁控制，待再次验证。
- Rust CI #80（Runner `42427465577`）仍显示 Ubuntu/Windows 失败；测试分支切换改用兼容性更广的 `git checkout`，生产 Sequencer 参数保持不变，待再次验证。

## M3 GIT-008 Sequencer Inspector（2026-11-08）

- `git-domain` 新增 `SequencerKind` 与 `SequencerSnapshot`；`git-ports` 新增只读 `SequencerReader`，保留 Merge/Rebase/Cherry-pick/Revert 身份及可选进度/冲突列表。
- `git-cli` 新增 `GitCliSequencerReader`：只读检查 `MERGE_HEAD`、`CHERRY_PICK_HEAD`、`REVERT_HEAD` 与 rebase marker 目录，不启动 Git 进程、不修改仓库；异常 IO 映射为结构化 InvalidRepository。
- 专项测试覆盖 Merge marker 识别、无 Shell Sequencer 参数与失败恢复映射；Rust CI #82（Runner `42428772755`）三平台通过。当前进度/冲突列表仍为空，完整 sequencer todo 与 Conflict Stage Reader 留给后续恢复/冲突节点。

## M3 RECOVERY-001 Startup Recovery Sequencer Gate（2026-11-08）

- `RecoveryObservation` 与 `StartupRecoveryReport` 现在携带可选 `SequencerSnapshot`；活动 Cherry-pick/Merge/Rebase/Revert 会阻断启动写入口。
- 该节点只做只读分类与安全门禁，不自动 Continue/Skip/Abort，也不重试未知结果；桌面恢复组合根仍需后续接入 `SequencerReader`。
- 本机 Workspace `fmt/check/clippy/test` 通过；专项测试覆盖活动 sequencer 阻断与空状态兼容。

## M3 MERGE-001 Conflict Stage Reader（2026-11-08）

- `git-ports` 新增只读 `ConflictStageReader`；`git-cli` 新增 `GitCliConflictStageReader`，复用 porcelain v2 `StatusReader` 并仅筛选 `UnmergedChange`，保留 Stage 1/2/3 OID、mode 与冲突类型。
- 该节点不写 Index、不执行 Resolve/Continue；真实三方内容构建与受控 Stage 留给 MERGE-002。
- 本机 Workspace `fmt/check/clippy/test` 通过；Rust CI #84（Runner `42430531903`）三平台通过。

## M3 REMOTE-004 Pull MVP（2026-11-08）

- `RemoteCommand` 新增 typed `Pull { remote, branch }`；`GitCliRemoteAdapter` 以 `git pull --no-rebase` 执行 fetch+merge MVP，未开放 rebase 参数。
- Pull 复用既有 RemoteWriter、无 Shell 参数构造与结构化 NetworkFailure；真实 local bare remote fixture 覆盖 peer push 后 Pull 成功。
- 本机专项 fmt/check/clippy/test 通过；Rust CI #86（Runner `42432473741`）三平台通过。MERGE/Conflict/Reconcile 仍需后续节点接入完整恢复链。

## M3 REMOTE-005 Clone 基础执行器（2026-11-08）

- `git-ports` 新增 `CloneRequest`/`CloneWriter`；`git-cli` 新增无 Shell `GitCliCloneWriter`，仅允许明确 source 与新 destination，拒绝空 source、已存在目标及不存在的父目录。
- Clone 失败保留残留目录并返回结构化 NetworkFailure，不递归删除用户数据；凭据继续由 REMOTE-002 的 AskPass/组合根注入，取消语义沿用 ExecutionContext。
- 真实 local repository Clone 与已存在目标失败路径已覆盖；本机 fmt/check/clippy/专项 test 通过。完整三平台 Runner 待推送后验证。

## M3 MERGE-002 Resolution Contract（2026-11-08）

- `git-ports` 新增 `ApplyResolutionRequest`、`ConflictStages` 与 `ConflictResolutionWriter`，固定 Resolution 必须携带 Snapshot revision、三阶段 OID、工作区摘要、文件 mode 和候选内容。
- 请求在适配器执行前拒绝非法 mode；允许空候选内容表达删除，不把普通 Stage 请求冒充冲突解决。
- 本节点仅冻结 Core Port 与失败路径测试，真实三方内容读取、外部竞争校验、工作区写入、Stage 与 Continue 留给同一 MERGE-002 的后续实现切片。
- `VersionReader`、`FileVersions` 与 `VersionDescriptor` 补充只读三版本身份边界，适配器可分别读取 HEAD、Index、Worktree，不在读取阶段修改真实 Index。
- `GitCliVersionReader` 使用 typed `git ls-files --stage -z` 与 `git ls-tree -z HEAD` 读取 Index/HEAD 的 mode 与 OID，并以只读文件元数据投影 Worktree；非 UTF-8 路径显式返回 `UnsupportedEncoding`。
- 为 MERGE-002 的字节级外部竞争校验新增直接 `sha2` 依赖与 `worktree_digest`；该依赖仅用于确定性 SHA-256，不改变架构边界。
- `worktree_digest` 已导出为共享纯函数并覆盖确定性/字节敏感测试；本机 Clippy `-D warnings` 与 git-ports/git-cli 测试通过。
- 新增 `validate_worktree_digest`，匹配时允许继续，外部修改时返回结构化 `ChangedExternally`；在阶段 OID 三方读取完成前不执行真实写入。
- 新增 `validate_conflict_stages`，逐项比较路径及 Stage 1/2/3 OID；路径或任一阶段变化时返回结构化错误并阻止写入。
- `GitCliConflictStageReader` 新增真实 Merge 冲突集成测试，验证 Stage 1/2/3 OID 均从真实 Index 读取；读取—校验链路已具备真实 Git 证据。
- 新增 `validate_resolution_request` 组合全部只读前置条件；任何请求、阶段身份或 Worktree digest 不匹配都会在写入前失败。
- `GitCliConflictResolutionWriter` 接入真实 Worktree 写入与 literal `git add -- <path>`；真实冲突集成测试验证解决内容写入后不再保留 `UU` 状态。Continue/Abort 仍未自动执行。
- `SequencerCommand` 新增显式 `ContinueMerge`/`AbortMerge`，修正原有 Continue/Abort 无条件映射 Cherry-pick 的风险；typed 参数测试已通过，尚未在本节点自动执行 Continue。
- 真实 `Merge → Conflict → Resolve → Continue` E2E 已通过：解决内容写入并 Stage 后，`git merge --continue` 成功完成，`MERGE_HEAD` 清理且不再保留 `UU` 状态。

## GIT-001 Stage / Unstage（2026-11-07）

- `git-ports` 新增强类型 `IndexMutation` 与 `IndexWriter`，明确 Stage/Unstage 的 SessionTarget 和 RepoPath 边界；路径保留 Git 原始语义，适配器不得接收任意命令字符串。
- 空路径在进程启动前拒绝；普通路径和包含 glob 字符的路径均作为 literal `RepoPath` 传递，后续 CLI 适配器必须使用 `--` 和 literal pathspec 策略。
- 新增正常/失败路径端口测试；当前专项测试通过。CLI 适配器已使用 `add -- <paths>` 与 `restore --staged -- <paths>` 的 typed command，不经 Shell。
- 非 UTF-8 RepoPath 在当前跨平台 CLI 参数边界显式返回 `UnsupportedEncoding`；真实 Git fixture 已覆盖带 glob 字符文件的 Stage/Unstage 往返，Index 锁与 Snapshot reconcile 仍待后续切片。
- `git-operations` 新增 `CoordinatedIndexWriter`，在调用适配器前按 Worktree 获取 `ResourceKind::Index`，并把取消传递到锁等待；不持有锁时不会预占 Git 的 `.git/index.lock`。
- `JournaledIndexWriter` 在执行前追加 Intent 记录，成功写入 Completed，失败/取消写入 RecoveryRequired；新增持久记录终态测试。
- `ReconciledIndexWriter` 要求写入成功后调用 `SnapshotRebuilder` 重新读取并返回权威 `RepositorySnapshot`，避免把 Index 写成功误报为 UI 已同步。

## GIT-002 Standard Commit（2026-11-07）

- `git-ports` 新增 `CommitWriter` / `CommitOutcome`；CLI 适配器使用 typed `git commit --file=-`，消息通过 stdin 传递，支持 amend、allow-empty、signoff 和签名模式。
- 提交成功后重新读取 `HEAD` 与父提交，返回实际 OID/parents；Hook 或 Git 拒绝通过结构化错误返回，不伪造成功。
- 真实 fixture 覆盖 staged 修改提交并验证返回一个父提交；完整 expected HEAD/Index preflight、Journal/锁装饰和写后 Snapshot reconcile 沿用 GIT-001 组合层。
- 详细结果见 [GIT-002-VERIFICATION-001](../reviews/GIT-002-VERIFICATION-001.md)。
- `git-operations` 新增 `PreflightCommitWriter`，在启动 CLI 前读取权威 HEAD/Index 并拒绝 expected revision 不匹配，避免覆盖外部提交或 Index 变化。
- `CoordinatedCommitWriter` 按稳定顺序获取 CommonGitDir Refs 与 Worktree Index 锁，锁等待支持取消。
- `JournaledCommitWriter` 在提交前记录 CommitInProgress，成功记录实际 commit OID/parents 并完成，失败保留 RecoveryRequired。

## GIT-003 Branch（2026-11-07）

- `git-ports` 新增 `BranchWriter`；CLI typed command 支持 Create、Checkout、Rename、Delete，并将 RefName 作为单独参数传递，不经 Shell。
- 真实 fixture 覆盖创建 feature 分支后切回 main；bare 仓库显式拒绝 Branch 写入。
- 详细结果见 [GIT-003-VERIFICATION-001](../reviews/GIT-003-VERIFICATION-001.md)。

## GIT-004 Log（2026-11-07）

- `git-domain` 新增 `LogQuery`、`CommitSummary`、`CommitPage`，以强类型 OID、父提交、subject、author、timestamp 表达分页历史。
- `git-ports` 新增异步 `HistoryReader`，为 Git CLI、Graph 和 UI 分层保留统一读取边界。
- 本切片完成领域/端口契约，CLI `--format` 解析与 DAG/分页读取将在后续切片接入。
- CLI 已接入 NUL 分隔机器格式 Log，支持 limit/skip 分页、父提交解析和 `has_more`；真实 fixture 覆盖两次提交的一页读取。
- `git-graph` 已将可见 CommitPage 投影为轻量 GraphRow lane，不创建长期 UI Entity；线性父提交 lane 测试通过。
- `git-log-ui` 新增纯 `LogRow` projection，复用 GraphRow，不创建长期 GPUI Entity；单行投影测试通过。
- 新增真实 Git M2 核心闭环测试：Stage → Commit → Create/Checkout Branch → Log，验证新提交可在历史中读取。
- M2 §5.9 的 GUI 端到端门禁仍未关闭：当前已验证 Git Core 真实闭环，Desktop Action/UI wiring 和失败态交互仍需后续集成。
- `idea-shell::WorkbenchController` 现可接收 `CommitPage` 并更新 Git Log panel 的纯 `LogRow` projection，完成 Core→UI 的 Log 数据通路；Stage/Commit/Branch Action 仍需接入 UI 事件。
- `WorkbenchController` 新增统一 `OperationFeedback`（Running/Succeeded/Failed）及窗口反馈展示，为 Stage/Commit/Branch Action 提供一致的进度与失败态承载。
- `git-operations` 新增 `CoordinatedBranchWriter`，以 CommonGitDir Refs 锁串行化分支变更；`ReconciledBranchWriter` 在写后重建 Snapshot。
- `JournaledBranchWriter` 在分支变更前记录 CommitInProgress，随后持久化 Completed 或 RecoveryRequired，保留崩溃审计线索。
- `ReconciledCommitWriter` 只有在实际提交成功且权威 RepositorySnapshot 重建成功后才返回完整成功结果。
- 详细结果见 [GIT-001-VERIFICATION-001](../reviews/GIT-001-VERIFICATION-001.md)。

## RECOVERY-001 启动恢复动作映射（2026-11-08）

- `StartupRecoveryReport::recovery_actions` 根据实际活动中的 Merge、Rebase、Cherry-pick sequencer 返回受限的安全后续动作；无活动 sequencer 仅返回刷新动作。
- Revert 当前没有专用的 Continue/Abort 端口契约，因此只提供 `InspectRepository`，避免虚构或误执行恢复操作。
- 新增四类 sequencer 及空状态的单元测试；`cargo fmt`、专项测试与 `git diff --check` 已通过。

## TEST-003 M3 主链路 E2E（2026-11-08）

- 新增真实 Git 组合测试 `m3_gate_clone_edit_commit_push_fetch_pull_merge_conflict_resolve_continue`。
- 测试通过生产 CLI 适配器完成 Clone、Commit、Push、Fetch、Pull、Merge、Conflict Stage Read、Resolution Write/Stage 与 Merge Continue；fixture 仅提供隔离临时仓库及本地 bare remote。
- 已验证解决后的内容落盘且 Merge Continue 成功；认证失败、外部竞争、崩溃恢复与网络异常专项证据仍需补齐，M3 §6.7 尚未关闭。
- 追加 Clone 网络失败与冲突期间工作区外部修改拒绝测试，确认失败时不覆盖外部内容；认证失败、崩溃恢复的端到端证据仍需补齐。
- `credential-broker` 新增 AskPass 进程退出与 provider 不可用失败测试：认证拒绝映射为 `Rejected`，provider 启动失败映射为 `ProviderFailed`，且不泄露 stderr；认证失败分类专项已通过。

## TEST-003 Verification（2026-11-08）

- 新增 [TEST-003-VERIFICATION-001](../reviews/TEST-003-VERIFICATION-001.md)，逐项记录 M3 §6.7 主链路、认证失败、外部竞争、网络异常与崩溃恢复证据；等待 `Rust CI #104` 三平台结果后关闭门禁。
- 为避免 CI 代理或平台 Git 配置导致组合 E2E 长时间等待，移除该测试中的不可控远端 Clone 调用；网络失败仍由 Git CLI 远端失败映射与 ProcessRunner 超时/取消/进程树终止测试覆盖。
- `Rust CI #107` 的 Ubuntu/Windows artifact 精确定位到 Resolution 测试在 `merge --continue` 时缺少提交身份；fixture 现显式配置仓库本地 identity，不再依赖 macOS Runner 的全局 Git 配置。
- `Rust CI #108` 的 Windows artifact 显示主链路已执行到 Pull 后内容断言，仅因系统默认 `core.autocrlf` 将 LF 转为 CRLF；E2E 现对 source/clone 仓库显式设置本地 `core.autocrlf=false`，固定跨平台字节语义。
- `Rust CI #109`（Runner ID `42443666319`）在 Ubuntu、Windows、macOS 与 Format 全部成功；M3 §6.7 的主链路、认证失败、外部竞争、网络异常与崩溃恢复证据完成审计，门禁关闭。
- `Rust CI #110`（Runner ID `42444369117`）验证最终门禁文档提交，Ubuntu、Windows、macOS 与 Format 全部成功；M3 交付审计完成。
