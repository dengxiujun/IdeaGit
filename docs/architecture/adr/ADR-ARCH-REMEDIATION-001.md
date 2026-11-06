# ADR-ARCH-REMEDIATION-001：第一轮架构阻断项收敛

- 状态：Accepted（D1–D7 已获项目负责人批准）
- 日期：2026-11-06
- 批准人 / 批准日期：项目负责人（本聊天明确批准）/ 2026-11-06
- 决策范围：设计文档及计划修正，不包含创建 Workspace 或功能实现。
- 依据：[架构审查](../../reviews/ARCH-REVIEW-001.md)、[接口冲突](../../reviews/API-CONFLICTS-001.md)、[M0 清单](../../plans/M0-EXECUTION-CHECKLIST.md)。
- 编号采用独立命名空间，避免与原设计内重复使用的 ADR-001 等编号混淆。

## 1. 问题背景

六份设计基线分别描述总体架构、Git Core、UI、Commit/Changelist/Diff、技术版本及开发计划，但共享类型、模块依赖、提交语义和里程碑尚未统一。当前没有可编译 Cargo 工程，审查发现是设计问题，不是已复现的软件缺陷。

根目录 AGENTS.md 第 3 节要求：阻断问题涉及模块边界、核心模型或已批准架构时先创建 ADR，项目负责人批准后才能实施。本草案把推荐决策和修订边界具体化，供批准；原文档与审查发现当前均保持原状态。

## 2. 阻断原因

| 阻断组 | 严重程度 | 审查追踪 | 具体原因 |
|---|---|---|---|
| Index/部分提交安全 | Critical | AR-01–06、AR-08、AR-14–15 | 真实 Index 写后恢复与临时 Index 策略冲突；外部竞争、Hook 副作用、剩余暂存内容协调未有可证明协议 |
| 编译依赖及身份 | High | AR-07、AR-09–11、API-01–13 | Application/Operations 共享命令可能反向依赖；Snapshot、选择、请求和返回值多版本；多 worktree/分组身份不足 |
| 状态机及恢复 | Critical/High | ST-01–05 | 已提交需恢复、未知结果、冲突等待和命令终态无统一映射；失败可能跳过 reconcile |
| 基线及计划 | High | AR-12、PLAN-01–12 | 固定工具链与 stable CI 冲突；Clone 缺任务；锁、Journal、恢复未成为写操作前置；Diff 和首切片错位 |
| 治理边界 | Medium | AR-13、AR-16 | Agent 危险操作禁令与产品能力边界不清；ADR 与子系统阶段编号冲突 |

## 3. 候选方案

### A：逐文件局部修补

保留各文档的同名模型，靠转换层兼容，安全细节留到编码阶段。改动量最小，但不能消除语义冲突，转换层也不能解决循环依赖和错误的 Index 协议。

### B：统一契约、分阶段安全门禁（推荐）

保留现有 Rust/GPUI-Kit/Git CLI 技术方向及候选版本；明确共享类型所有权、统一操作结果与恢复，修正主计划依赖。高级写功能在安全协议和真实仓库验证完成前不开放。

### C：重构技术栈或写引擎

更换 UI/运行时或以 libgit2 替代 CLI 写入，并重新设计模块。需要重新证明 Hooks、过滤器、签名及平台兼容性；现有证据不足以支持这种扩大范围的变更。

## 4. 方案对比

| 维度 | A | B | C |
|---|---|---|---|
| 对原选型影响 | 小 | 不换技术栈，调整契约归属 | 大 |
| 消除接口冲突 | 不彻底 | 建立唯一契约 | 仍需重新建立契约 |
| 数据安全 | 风险推迟 | 门禁前置，未知能力禁用 | 重新验证全部写入语义 |
| M0 可执行性 | 容易产生返工 | 可拆分为设计、骨架、验证 | 工期和范围显著扩大 |
| 可验证性 | 缺统一标准 | 文档一致性与运行验证分开 | 验证成本最高 |

## 5. 推荐方案及拟批准的具体决策

### D1：固定编译依赖与契约归属

箭头统一表示“左侧编译依赖右侧”，执行调用图必须另行标注：

- git-domain：纯值对象、Snapshot、选择/Diff/Commit 请求与结果、命令 DTO、结构化错误；不依赖 Tokio、GPUI、文件系统实现。
- git-ports → git-domain：读写、Journal、凭据、存储、监控端口及 ExecutionContext；Tokio 取消/进度类型可放此层，不进入 Domain。
- git-diff、git-changelist、git-graph → git-domain；需要外部持久化时通过 git-ports 注入，不反向依赖 Application。
- git-commit → git-ports/git-diff；git-operations → git-ports/git-commit；git-application → git-operations/git-ports。
- adapters → git-ports/git-domain；禁止反向依赖 git-application。
- UI → git-application 和共享 git-ui-model；Desktop 负责具体实现装配。Shell 与 Feature 不相互导入具体实现。

GitCommand 的所有权放 git-domain，Application 可重导出；ChangeSelection/Diff DTO 同样放 domain，算法留在对应 crate。git-commit 按 CCD 纳入架构清单，创建时机随实际任务决定。本决策不授权现在创建任何 crate。

### D2：统一身份与最小业务模型

- RepositoryId 表示规范化 common Git dir 所属逻辑仓库；WorktreeId 表示具体 worktree。Session 以二者组合定位；bare 会话没有 WorktreeId，仅开放适用读取能力。
- Facade 打开仓库返回会话句柄（携带 repository、可选 worktree、会话世代）；后续命令/查询使用同一目标，避免只传 RepositoryId 时无法区分 linked worktree。关闭句柄释放该使用方引用，不直接关闭其他窗口的会话。
- RepositorySnapshot 使用 repository、可选 worktree、revision: SnapshotRevision、head: HeadState、operation、changes、untracked、ahead/behind: Option<u64>；bare 缺工作区能力不等于干净工作区。
- HeadState 统一 Unborn/Attached/Detached；RepoPath 字节私有，经验证构造，补齐共享类型需要的 Eq/Hash/序列化契约。
- Changelist 与绑定显式持有 repository/worktree/list 身份、schema 和基线指纹；save 包含目标作用域，空列表仍可正确归属。
- UI 树节点键含会话目标、分组、路径及比较端点；Draft 按 worktree/模式区分，多窗口共享时用版本检查避免覆盖。

### D3：统一 Facade 与执行上下文

Facade 定义 open/close、snapshot、log、commit_details、diff、submit、cancel、operation 和 subscribe；高级能力另含 prepare_commit、编辑 Index 会话入口，通过 capability 判断是否开放。所有真实写入仍经 submit 和 Coordinator。

snapshot 返回 Arc<RepositorySnapshot>；事件统一 GitEvent，只携带目标、版本/OperationId 等轻量数据；订阅建立后读取当前快照，缓冲期间事件按 revision 去重，lag 时重新同步；操作终态可独立查询。

GitCommand 包括标准 Index Commit 与 Selection Commit 两种明确请求，文件/选区 Stage、Unstage、ApplyEditedIndex 以及 Sequencer Continue/Skip/Abort。请求只保留一个权威执行目标；重复目标不一致必须拒绝。

Commit 共有选项明确 message: Vec<u8>、amend、allow_empty、signoff、author/committer 可选覆盖、signing；默认尊重 Git Hooks，不提供隐式跳过策略。Selection 请求另含选区指纹与预览身份，不能复用 selected_paths 偷换语义。

ExecutionContext 统一 operation_id、cancellation、progress、deadline；所有长操作/写端口显式接收。ProcessSpec 补 env、stdin、输出限额及退出状态，敏感字段禁止直接 Debug/日志输出。

DiffQuery 使用统一 ContentSource 和资源上限；比较目标含不可混淆的会话身份。ConflictStage 需有效 stage 和文件路径。展示 Diff 与可写 Patch 分型，后者必须有完整原始字节、基线及目标 Blob 校验。

### D4：操作状态与恢复结果

命令生命周期为 Queued → ResourceWait → Preflight → Running → Reconciling → Finished；运行中的凭据等待单独表达。执行前取消且确定无副作用可以直接结束，其余失败/取消均核对仓库后产生结果。

Sequencer 的持久状态与单次命令生命周期分开。出现冲突时命令完成为 Conflict，仓库保留 AwaitingConflictResolution；Continue/Skip/Abort 创建关联命令，不让等待用户的操作长期占用执行队列。

Commit 结果统一为 CommittedAndReconciled(OID/Tree)、CommittedButNeedsRecovery(OID/Journal)、NoCommit(reason)、Indeterminate(Journal)。只有确认未发生提交才可返回 NoCommit；后两类不能自动重复提交。UI 显示同一语义，成功清理 Draft 需匹配原提交草稿版本。

Journal phase 采用 CCD 的 IntentRecorded、SnapshotCaptured、TempIndexPrepared、PreviewReady、PreflightRevalidated、CommitInProgress、CommitObserved、ReconcilingIndex、Completed、RecoveryRequired；它与对外 OperationPhase 是映射关系，不强行使用同一枚举。日志包含 schema、旧 HEAD、强 Index 指纹/备份、候选 Tree、实际 OID/父提交、阶段和恢复结果；不可逆操作前保证所需意图耐久化。

Save Merge Result 仅保存草稿；Apply 校验冲突 Stage OID、工作区指纹和候选内容后执行写入与暂存；Continue 仍为独立命令。

### D5：写入安全边界与高级能力门禁

废弃 CORE 的 stage_selected 写真实 Index 后恢复方案。M2 只开放标准 Index Commit；Changelist 是独立元数据，不能隐式 Stage。

区分应用资源锁、Git 自有锁和候选 Index 锁。普通 Git 写命令由 Git 获取自身锁，应用不能预占真实 index.lock 后调用同样需要该锁的命令。高级候选 Index 发布协议必须有独立实现规格、锁内比较/发布和跨平台证明；不能把 check→apply 或进程内 Mutex 宣称为外部竞争防护。实现协议前保持相关能力关闭。

部分提交以 HEAD 基线构造候选并预览；记录 H0/I0/W0/C，精确定义剩余 staged 变更向 H1 的转移。重叠、抵消、重命名、mode、filter、split/sparse Index 等未验证组合明确拒绝或只读降级。禁止原样复制候选覆盖真实 Index，也禁止恢复备份覆盖外部新修改。

标准 git commit 与预览不是统一原子事务；不得声称后验检查可阻止 Hook/外部 HEAD 变化已经造成的提交。候选 Tree/父提交保证未被证明的高级组合保持关闭；验证失败或观察到偏离进入带实际 OID 的恢复状态。本 ADR 批准的是安全边界与后续证明任务，不是宣告事务协议已解决。

路径采用命令适用的 literal pathspec 策略；NUL 和 -- 不能替代 literal 语义。ignore-whitespace、textconv、截断或不完整 AST 不能作为写 Patch 来源。保留过滤前后字节区别，避免二次 clean filter。

### D6：保留候选版本，调整验证与计划门禁

保留 TECH 当前 Rust 1.99.1、Edition 2024、resolver 3、GPUI-Kit =0.9.0 候选，不在此 ADR 升降版本。CI 显式使用固定工具链；MSRV 未测试时标为未验证，不推定等于日常版本。Git 最低版本和其他依赖 patch/features 由 INFRA-002 形成核验提案；若需改变核心基线另提 ADR。

主里程碑以修订后的 IP 为准，Core/UI/CCD 子阶段使用独立命名并映射，避免相同 M 数字被当成同一承诺。拟调整：

| 阶段 | 具体修订 |
|---|---|
| M0 | INFRA-004 架构决策部分前置；候选选择→INFRA-001 骨架→INFRA-002 锁定→INFRA-003/ UI-001 三平台验证；UI Spike 加 Dock/焦点/列表/编辑器/任务生命周期 |
| M1 | CORE-002 联调依赖 CORE-003；Session 依赖 Discovery；最小只读 DIFF-001/002 前移 M1/M2，满足首条切片 |
| M2 | GIT-001/002/003 功能验收依赖 CORE-011/012；新增 RECOVERY-000 最小 Commit/Index 启动恢复，依赖 Journal；UI-007 依赖 UI-006，UI-009 依赖 UI-003 |
| M3 | 新增 REMOTE-005 Clone（Executor/凭据/取消/目标路径安全）；Fetch 联调依赖 REMOTE-002；Pull MVP 明确 fetch+merge，依赖 Merge/Recovery，rebase 模式待对应执行器完成 |
| M4 | UI-013 真功能开放依赖 CCD-007/TEST-004/恢复；新增 CCD-008 EditableIndexSession/Apply、CCD-009 Preview 生命周期；CCD-004/006 补 Lock/Journal/Commit/Patch 前置 |
| M5 | REBASE-004 加执行器/计划执行前置；SHELF-002 加 Coordinator/Journal/Recovery；UI-015 加 GIT-007；E2E 列出完整任务 ID |
| M6 | 安全和故障测试随写功能前置，M6 为全量复核；打包可并行，发布必须依赖 QA/安全/迁移门禁；替换 UI-M5 等非任务 ID 引用 |

新增 ID 须在落文前查重，已有 ID 不复用。CORE-010 的接口/调度实现与依赖锁/Journal 的可执行验收分开，不用简单相互添加 depends_on 造成新任务环。所有“相关任务”在修订时展开为明确 ID。

### D7：治理与范围

原有 ADR 表视为历史决策摘要，修订时指向正式 ADR 文件；废弃未定义 S0–S3 表述，映射到已命名阶段。Agent 不得在用户真实仓库运行 reset --hard 的禁令保持不变；产品 Reset 能力需明确模式及独立确认/fixture 验证，本草案不授权执行危险命令。

## 6. 影响范围与批准后执行

| 文件 | 批准后拟修改内容 |
|---|---|
| docs/architecture/ARCH-001-overview.md | 编译依赖图、共享模型、命令/事件、安全边界、主里程碑和 ADR 引用 |
| docs/architecture/LLD-GIT-CORE-001.md | 去除真实 Index 恢复捷径；统一 Ports/Facade/Session/状态/Journal |
| docs/architecture/LLD-UI-001.md | Session/请求键、Changes Tree 分组身份、订阅/草稿、提交和冲突结果映射 |
| docs/architecture/LLD-CCD-001.md | DTO 所有权、提交结果、写入门禁、候选协议待验证边界和阶段映射 |
| docs/architecture/TECH-BASELINE-001.md | 固定工具链与 MSRV 区分、依赖核验状态、待验证清单，不改变候选版本 |
| docs/plans/IP-001-development-plan.md | 前置依赖、新任务、分阶段开放与测试门禁、规范接口示例 |
| docs/plans/M0-EXECUTION-CHECKLIST.md | 跟踪 ADR 决定、未完成验证及放行条件 |
| docs/plans/PROGRESS.md | 记录批准、文档修订、仍未实现/未验证事项；不标记 M0 已完成 |

批准后按“契约与依赖→安全与状态机→UI 映射→计划与基线→交叉检查”修订。本草案的批准仅授权上述文档修正；不意味着三平台测试通过、功能开发获准或所有审查问题关闭。

如负责人只批准部分决策，则仅修订对应范围；存在前置依赖的决策需同时批准。D2/D3/D4 依赖 D1，D5 的落文与状态映射依赖 D4，D6 依赖相应已批准的安全门禁。

## 7. 验证标准

### 本轮草案检查

- 七项必需内容齐全，全部状态标为 Draft，无虚构批准记录。
- AR/API/ST/PLAN 阻断组可追踪到 D1–D7，候选方案有可比较影响。
- 不改原设计、计划、PROGRESS 或代码，不创建 Workspace。

### 批准后的文档验收

- 六份设计中共享符号只有一套权威定义，其余引用或示例与其一致；示例明确是否完整可编译。
- 编译依赖表可拓扑排序，纯 Domain 不引入 UI/运行时；不把文档 DAG 当作实际 Cargo 证明。
- 命令失败/取消/冲突/未知结果/已提交待恢复均有 UI 映射和允许动作。
- 真实 Index 写后恢复捷径移除；外部竞争、锁所有权、Hook 及过滤器限制没有被省略。
- 每个 IP 依赖均指向存在的任务，新增 ID 唯一，任务图无环；功能开放依赖安全测试，不仅依赖 UI 完成。
- PROGRESS 区分“设计冲突已修订”“协议待验证”“功能未实现”；审查历史不抹除。
- Markdown 链接、章节、diff 空白检查通过。

### 后续工程验收（本次不运行）

Workspace 建立后才运行 cargo metadata、fmt/check/clippy/test 与三平台启动；写能力开放前运行真实 Git fixture、外部多进程竞争、Hook/签名失败、逐阶段崩溃恢复、同文件 staged/unstaged 重叠及特殊路径/过滤器矩阵。未完成这些测试的能力不得标记为安全可发布。

## 8. 批准记录

2026-11-06：项目负责人回复“批准，但是先把 adr目录移动到architecture下面”，明确批准 D1–D7。已先将目录迁移至 docs/architecture/adr；后续按批准范围修订设计、计划与进度。批准不等于工程验证通过。

本文保留候选方案及批准前的提案措辞作为决策背景；本节和文首状态为当前有效批准记录。
