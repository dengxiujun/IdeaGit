# API-CONFLICTS-001：跨文档接口与状态机冲突

日期：2026-11-06。严重程度、文件简称同 [架构审查](ARCH-REVIEW-001.md)。以下全为审查建议，不构成对原设计的修订或接口冻结。

## 1. 类型与接口差异

| ID / 程度 | 关联位置 | 冲突 / 原因 | 建议 |
|---|---|---|---|
| API-01 High | ARCH §5；CORE §3；CCD §4；IP §4.2 | SnapshotVersion/version 与 SnapshotRevision/revision；repository_id/worktree_id 与 repository/worktree；staged/unstaged 与 changes；ahead/behind u32/u64 | 冻结一个 canonical Snapshot，UI 分组作为投影；统一整数与字段；保留刷新一致性标记 |
| API-02 High | ARCH §5.2；CORE §3 | HeadState::Branch{name} 与 Attached{branch}；RepositoryInfo 必有 worktree_root 而 Location 可 None/is_bare | 分开仓库发现位置与领域 HEAD；bare 只读能力显式表达；不能用空路径代替缺失工作区 |
| API-03 High | ARCH §6.1；CORE §4；CCD §5.1 | discover 返回 Info/Location；reader 参数 RepoId/Location；History commit/commit_details；DiffReader diff(repo,q)/diff(q) | Facade 使用稳定 ID，adapter port 使用解析后 Location；避免同名不同层契约；统一方法名与返回 DTO |
| API-04 High | CORE §11；IP §10.2；UI §6/8 | snapshot 返回值/Arc 不同；CORE 无 log/operation；两者无完整 cancel/详情/preview/selection 接口；GitDomainEvent/GitEvent/OperationFinished 名称不一致 | 建立 Facade 必需方法和事件表；明确 subscribe 初始快照、broadcast lag 后 resync、终态查询和关闭生命周期 |
| API-05 Critical | CORE §8；CCD §4.5；IP §5.5 | CommitRequest 三种不兼容字段：String/Vec<u8>、sign/signing、paths/selections、revision 缺失；同名 Outcome struct/enum；无法表达同一提交语义 | 分离标准 Index 请求与 Selection 请求，明确 author/committer/hooks/signing 策略；一致传递预期状态；repo 双重携带须拒绝不一致 |
| API-06 High | ARCH §6.3；CORE §5；CCD §5.2；UI §11.5 | GitCommand 没有 StageSelection、ApplyEditedIndex、Skip 等；ChangeCommand 未声明如何进入 Facade | 冻结强类型命令封装/转换；禁止 UI 旁路 ChangeCommandPort；为未实现能力返回明确不支持 |
| API-07 High | CORE §4；CCD §5.2；IP §4.3 | ExecutionContext deadline 有/无；多数 port 未携带 ctx；ProcessSpec 缺 env/stdin，无法表达临时 Index/消息/AskPass | 统一 ctx 与进程配置；显式 env 覆盖及敏感字段策略，stdin 流式/bytes，超时和取消语义一致 |
| API-08 High | ARCH §6.2；CORE §10；CCD §11.1 | DiffEndpoint 与 ContentSource；WorkingTree/Worktree；query repository/max_bytes 有无；冲突端点粒度不同 | 固定 DiffQuery、path+stage 约束、输出上限/截断标记；读展示选项不能直接进入 Patch 写协议 |
| API-09 High | ARCH §9.2；CCD §4.3；IP §7.3/10.1 | ChangeSelection path 与 FileIdentity、LineRanges/Lines、hunk_ids/ids/ranges 不同；domain/changelist 归属未定 | 将共享选择 DTO 放下层避免环；固定 Hunk 身份及指纹、行号基准、范围合法性和重新确认规则 |
| API-10 High | CORE §3；CCD §4.2；UI §9.3 | CORE ChangeKind 有 Unmodified，CCD 无；CORE RepoPath 无完整定义而 UI 树需 Hash；CCD 公开 Vec 可绕过构造校验 | 明确这是不同 DTO 还是统一枚举；私有路径字节+验证构造器；补 Eq/Hash/Serde 策略，避免复制后缺 trait |
| API-11 Medium | CORE §4；ARCH §14.2；CCD §14 | GitUnavailable/GitExecutableUnavailable 等错误名不同；CCD 错误无法无损映射通用错误 | 统一错误码、retry 分类与诊断结构；恢复结果携带 commit OID/journal ID，禁止降成单一 Failed(String) |
| API-12 High | IP §10.1；CCD §3；CORE §2 | GitCommand/CommitRequest/Selection/Diff 类型归属可能反向依赖；IP Workspace 缺 git-commit | 先完成 crate 所有权表与拓扑，再创建成员；暂未使用模块不必立即拆 crate |
| API-13 Medium | UI §9.4；CCD §8.3/12.5 | Draft 只按 repository 保存，CCD 要两模式选择和草稿隔离；WorktreeId 与多窗口身份未加入 | 明确 draft key、保存版本/冲突策略和成功清理条件；同仓库多窗口不能相互清空输入 |

## 2. 状态机冲突与必要统一

| ID / 程度 | 关联位置 | 原因 | 建议与验收 |
|---|---|---|---|
| ST-01 High | CORE §7 enum/图；IP §5.2 | enum 无 ResourceWait；图使用 Failed/Completed 而 enum 用 Finished(outcome)；Running→Failed 跳过 reconcile | 一份权威转移表：所有可能有副作用的退出先核对实际状态，再产生终态；明确队列取消例外 |
| ST-02 High | CORE §7；ARCH §7.2 | 冲突既是 Active(AwaitingConflictResolution) 又可 Finished(Conflict)，Continue/Abort 用新命令却未定义原 operation 身份 | 分开命令 operation 与持久 sequencer session；明确父子关联、锁释放及何时允许 Continue，避免等待用户时占队列导致死锁 |
| ST-03 Critical | CORE CommitOutcome；CCD §4.5/9.4；UI §9.2/CCD §12.2 | CommitOutcome 与 PartialCommitFinalOutcome 区分粒度不同；UI 只有 Failed/Running 的版本不能展示已提交需恢复或未知结果 | 统一 CommittedAndReconciled、CommittedButNeedsRecovery(OID)、NoCommit、Indeterminate；每种 UI 文案、Draft 清理、允许重试动作都测试 |
| ST-04 High | CORE §7；CCD §9.4；IP §7.5 | IntentRecorded/TempIndexPrepared 与 Captured/CandidateIndexCreated 等 phase 不一致；IP 缺预览确认和持久意图阶段 | Journal 持久 phase 与对外 operation phase 分层，建立映射和 schema 版本；每个不可逆边界崩溃恢复测试 |
| ST-05 High | UI §12；IP §6.5；CCD §13 | Save Result、Apply、Stage、Continue 的状态尚未显式联动；仅 revision 不能保证文件没被外部编辑 | Apply 携带 stage OID、目标工作区强指纹、候选内容和 mode；Saved 不等于 Resolved，Resolved 不等于 Sequencer 完成 |

## 3. Rust 示例的可编译性标记

所有六份文档均不是完整 Rust 工程；不能整篇抽取代码块后直接编译。以下区分语法片段、待定义接口和运行时占位，避免把 todo! 错判成语法错误。

| 关联文件 | 范围 / 严重程度 | 缺口与建议 |
|---|---|---|
| ARCH | §5–7、§9–10、§13–14，Medium | 独立 IDs/enum 可在依赖和上下文补齐后使用；大量 HeadSnapshot/GitError/LogQuery 等未定义、未导入；仅作为领域/API 草案 |
| CORE | §3–11，High | Uuid/Serialize/Path/async_trait/ArcSwap 等导入或依赖未齐；StatusData 等未定义；同文 diagram 和 enum 不一致；“可直接指导编码”不等于可直接编译 |
| UI | §4–13，Medium | Idea*/Git*、ViewModel 和布局记录为自定义类型；缺 Render、Panel/renderer 实现、初始化、订阅及 GPUI Context 生命周期；没有完整启动示例可验证 |
| CCD | §4–5、§11–14，High | DTO/API 草案，依赖导入/Serde/BlobReadHandle/EditableIndexSession 存储未全；流程图不能替代并发与恢复算法；部分最小 enum 本身合法 |
| IP | §3.5、§4–10，High | TestRepository 的 todo! 在类型补齐后能编译但调用会 panic；ProcessSpec 还不满足 CLI 需求；各阶段 DTO 不可与其他文档同名类型同时粘贴 |
| TECH | §3–5，Medium | 只有 TOML 示例，没有 Rust 源码；workspace 字段须由成员继承；依赖范围并非已解析锁文件，目录通配不能替代成员 manifest |

M0 只需先决定公共契约所有权与必需字段；后续独立任务中以最小 contract crate/测试逐个验证，而不是本轮创建工程。

