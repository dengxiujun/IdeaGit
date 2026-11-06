# ARCH-REVIEW-001：架构与数据安全审查

日期：2026-11-06。范围：AGENTS.md、五份 architecture 文档及 IP-001；辅助阅读 CODEX-GUIDE-001、PROGRESS.md。结论：**设计需先收敛，不宜直接按所有示例并行实现**。本次仅新增三份审查/执行清单，未修改设计基线、源代码或创建 Cargo Workspace。

## 1. 证据与结论边界

以下简称均对应仓库真实文件：ARCH = docs/architecture/ARCH-001-overview.md；CORE = docs/architecture/LLD-GIT-CORE-001.md；UI = docs/architecture/LLD-UI-001.md；CCD = docs/architecture/LLD-CCD-001.md；TECH = docs/architecture/TECH-BASELINE-001.md；IP = docs/plans/IP-001-development-plan.md。定位采用章节和符号，避免文档增删后行号失效。

严重程度：Critical（可能丢失/误提交数据，阻断对应写功能）；High（阻断契约或工程落地）；Medium（实现前须补全）；Low（文档治理）。发现是设计冲突或缺口，不是已经观察到的运行故障。

仓库无 Cargo.toml/Cargo.lock 和 Rust 源工程；因此未运行 Cargo metadata/check/clippy/test、未安装依赖、未做三平台窗口或故障注入测试。无实际依赖图可用于证明“Workspace 无环”。既有 CodeGraph 为空代码索引，本次审查对象为文档，无需通过符号索引定位代码。

## 2. 架构与安全发现

| ID / 程度 | 关联文件与位置 | 原因与影响 | 建议及验收 |
|---|---|---|---|
| AR-01 Critical | CORE §8；CCD §9.1–9.3；ARCH §9 | CORE 要求 stage_selected 写真实 Index 后恢复，与 CCD 禁止的捷径直接冲突；同文件 staged/unstaged 共存可误提交或覆盖原 Index | 通过 ADR 淘汰该捷径；M2 仅标准 Index Commit；部分提交采用 CCD 候选流程，但必须先完成下列协议证明 |
| AR-02 Critical | CCD §7.2、§9.3.3；IP §7.2 | “锁下校验后 git apply”未定义锁所有权：进程内锁挡不住外部 Git；若持有真实 index.lock 再调用需获该锁的 Git，则会冲突；--check 与 apply 分开执行留下竞态 | 区分应用锁、Git 锁、候选 Index 锁；设计候选生成及真实 Index 独占锁内比较/发布协议，不能提前删锁让子进程运行。验证外部写入发生于每一间隙时均不覆盖 |
| AR-03 Critical | CCD §9.3.1–9.3.4、§10.3 | HEAD 预验与标准 git commit 之间仍可变更；候选从 H0 构建却可能基于不同父提交提交。提交后检查不能撤销已造成的历史变化；Hook 修改候选也可能在提交后才被发现 | 明确标准 Commit 的竞争保护边界、允许 Hook 变化策略及不支持组合；记录实际父提交/Tree/OID；不声称外部 old-value update-ref 能包裹 git commit；无法证明安全时限制高级提交 |
| AR-04 Critical | CCD §9.3.3、§16.2 T-005 | “剩余 staged 变更转移”缺少重叠编辑的可执行定义，保留 I0 或用 C 覆盖都不等于保留暂存意图 | 定义 H0/I0/W0/C 到 I1 的逐内容及 mode 语义；同文件 A 已暂存、B 仅选中、A/B 重叠/抵消/重命名须有预期 Blob；无法无歧义转移进入恢复，不自动猜测 |
| AR-05 High | ARCH §6.4；CCD §7.1；IP §5.4 | -- 分隔选项但不关闭 Git pathspec 通配和 magic；文件名含 *、? 或 :(...) 可能匹配额外文件；NUL 只解决分隔 | 使用命令适用的 literal pathspec 策略，验证参数/环境作用范围；加入星号、方括号、冒号 magic、前导短横、换行和非 UTF-8 fixture。依据 [Git pathspec](https://git-scm.com/docs/gitglossary) |
| AR-06 High | ARCH §3.3；CCD §4.4、§7、§11 | 展示 Diff 可忽略空白/textconv/截断，不能保证可逆或可用于写 Patch；AST 也明确缺完整 headers | 分离展示 Diff 与可应用 Patch；写入必须完整、原始字节、保留 mode/rename/no-newline，禁用展示转换；truncated/未知类型只读。--check 不替代目标 Blob 核验，参见 [git apply](https://git-scm.com/docs/git-apply) |
| AR-07 High | CCD §4.3、§5.1、§8.1；CORE §6 | Changelist 声明须含仓库/worktree/指纹，但 Changelist/save(list) 未带所属仓库且空列表无法推断；多 worktree 的 RepositoryId 语义未冻结 | 持久化键显式包含 repo/worktree/list，绑定含指纹与 schema；事务更新、迁移和备份；明确共享 common-dir ID 与每 worktree Session ID |
| AR-08 High | CORE §7；CCD §9.5、§13；IP §6.6 | CORE Journal 仅 before_index_stat 不足以保全内容；异常结果、持久化顺序和恢复记录字段不统一 | 统一 Journal schema，保存强指纹、备份、候选树、已观察 OID、阶段；写前耐久化；崩溃后不自动覆盖当前 Index，不因返回丢失重复 Commit |
| AR-09 High | CORE §2.1；ARCH §1.3；IP §10.1；CCD §3 | 箭头混用执行调用和编译依赖；GitCommand 归 Application 而 Operations 消费它可形成双向依赖 | 采用下一节单向方案，契约独立放低层；禁止适配器依赖 Application；最终以 metadata 验证 |
| AR-10 High | UI §9.3；CORE §3 | 同文件可同时位于 Staged/Unstaged，而节点 File(RepoPath)、Directory(RepoPath) 不含分组；选择、展开和缓存键会碰撞 | 节点身份加入 worktree/group/比较端点；验证同路径两个节点独立操作 |
| AR-11 High | CORE §11；UI §6、§11.5；IP §10 | UI 所需 log/details、cancel、subscribe、preview、hunk/edit-index 和 operation 结果不能由现有共同 Facade 完整表达 | 先统一 API-CONFLICTS-001 中契约；增加订阅丢消息后全量同步、多窗口关闭引用计数和请求世代处理 |
| AR-12 High | TECH §3、§4；IP §3.4 | 固定 1.99.1 与 CI action@stable 冲突；workspace.package.rust-version 写成日常工具链却未说明真实 MSRV；原生系统依赖尚未落实 | CI 显式选定工具链；单独标注已测 MSRV；固化 OS/target/SDK/图形会话，不能把 cargo check 当窗口验证 |
| AR-13 Medium | AGENTS §4；ARCH §14.1；IP GIT-007 | Agent 禁止在用户仓库执行 reset --hard，产品规划却包含确认后的 Reset；执行规范与产品能力边界未说明 | ADR 说明开发过程禁令和产品能力是否分别适用；本轮不执行危险操作；仅在隔离 fixture 验证获批准的产品语义 |
| AR-14 Medium | CCD §9.3、§11.6；TECH §6 | 临时 Index 初始化、split/sparse Index、extensions、skip-worktree/intent-to-add、unborn/bare、clean filter 二次处理缺细则 | 指定支持矩阵；临时空文件不能未经初始化就当合法 Index；保留或安全拒绝未知扩展；区分 Blob 字节和 worktree 字节，特殊类型整体处理 |
| AR-15 Medium | CORE §5、§12；IP §4.3 |取消 token/丢弃 future 不保证终止 Git 及 hook/ssh 子进程树；读 Status 也可能有可选 Index 刷新锁 | 明确进程组/Windows Job 等平台协议、排空管道和 wait/reap；读取考虑 GIT_OPTIONAL_LOCKS 策略；取消后始终 reconcile |
| AR-16 Medium | ARCH §17；IP §13.2；UI §18；CCD §18 | ADR 编号在不同文档表示不同决策，CCD 末尾 S0–S3 未定义；不能据此建立唯一追踪 | 统一命名空间和决策索引，补阶段映射；此处仅指出，不改原稿 |

## 3. Workspace 循环依赖审查

**未确认存在实际 Cargo 环；确认存在按草案拼接后形成环的风险。**

- IP §10.1 将 GitCommand 放 git-application，CORE 使 Application 调 Operations：若 Operations 引用该命令，则 application → operations → application。
- IP 把 CommitRequest 放 domain、ChangeSelection 放 changelist；若 Request 包含 selections 且 changelist → domain，则 domain → changelist → domain。
- domain 中定义的 Diff DTO 若引用 git-diff 的类型，而 git-diff → domain，也会形成类似环。以上都是条件性风险，不是现存源码事实。
- CCD 已提醒 git-commit 不反向依赖 operations；应保持此约束。UI Shell/Feature 间用低层 Panel/Action 契约或启动层装配，避免互相导入具体 Entity。

建议的编译依赖（待 ADR）：domain 为纯类型；ports → domain；diff/changelist → domain；commit → ports/diff；operations → ports/commit；application → operations/ports；adapters → ports/domain；UI → application/共享 UI model；desktop 装配 Application、Adapters、UI。ExecutionContext 可位于 ports 或独立执行契约层，Tokio 类型不得进入 domain。Journal/Watcher/Credential 使用注入端口。

## 4. 依赖与实际 API 核验

以下为 2026-11-06 在线资料核对，不是安装/编译通过证明。latest 页面记录了本次显示版本，后续须改用锁文件和对应发布源码追溯。

| 核验对象 / 关联文件 | 结果与建议 |
|---|---|
| Rust 1.99.1（TECH §1） | 官方发布目录列出该版本，候选版本非虚构；未安装验证。[Rust releases](https://blog.rust-lang.org/releases/) |
| Edition 2024 / resolver 3（TECH §4） | 组合有效；虚拟 workspace 应显式设 resolver，依赖自身 resolver 不决定顶层解析。[Edition Guide](https://doc.rust-lang.org/edition-guide/rust-2024/cargo-resolver.html) |
| GPUI-Kit 0.9.0（TECH §1/5，UI §3） | 发布 manifest 确认 gpui-pre、macros/platform =0.3.11；base/component/assets 要求 0.9.0；fast 要求 0.1.13，后两类不是精确锁。test-support/gpui-fast feature 均存在。[发布清单](https://docs.rs/crate/gpui-kit/latest/source/Cargo.toml) |
| GPUI 导入与平台 | Kit 重导出 GPUI，避免另加不匹配 gpui。官方安装页当前要求 Rust ≥1.92、macOS ≥15、Windows MSVC/C++ 工具链；Linux 需原生库及可用图形/Vulkan 环境。项目支持下限仍待实际验证。[安装指南](https://gpui-kit.com/docs/installation/) |
| Dock（UI §4.3） | DockArea、PaneTree、Panel、PanelView 为实际 API；PaneTree 为值模型，非 Entity 树。IdeaDockSkin 是自定义适配，须实现锁定版本 renderer 契约，布局恢复需重建面板身份。[Dock](https://gpui-kit.com/base/dock/) |
| Tokio 1（CORE §5、TECH §2） | 官方当前 API 1.54.1 有 process::Command，需 process feature；设计方向可实现。取消需显式管理 child，不能仅 drop future。[process](https://docs.rs/tokio/latest/tokio/process/index.html) |
| tokio-util 0.7（CORE §16、TECH §4） | 当前 0.7.20 有 sync::CancellationToken；不能仅因 CORE 写 rt 就断言 CancellationToken 本身必须 rt，当前模块页 sync 无该 gate，task/context 才标 rt。冻结实际 patch 后按所用 API 逐项验证。[模块表](https://docs.rs/tokio-util/latest/tokio_util/index.html) |
| notify 8（CORE §6） | 已核对 8.2.0 文档；原生事件不等于可靠快照，网络文件系统可能需要 PollWatcher；debounce 不由基础 watcher 自动保证。[notify](https://docs.rs/notify/8.2.0/notify/) |
| git2 0.20 | 可选优化尚未落地；本次指定 0.20.2 API 页面访问失败，未完成 patch/API 核实。不能宣布 SHA-256 与 CLI 等价；M0 不引入可选适配，未来能力测试通过后启用 |
| async-trait/thiserror/serde/serde_json/uuid/tracing/tracing-subscriber/tempfile/proptest | 文档仅给 major/minor 范围，无完整解析版本；本次未逐个确认 patch/MSRV。uuid v4/serde 和 serde derive 已列；DTO 自身 derive/导入仍不完整。归入 INFRA-002 核验清单，不能标全部兼容 |
| arc-swap/parking_lot/sha2/secrecy/SQLite | CORE、CCD、IP 示例用到但统一 TECH 清单不完整或明确待选；应依据实际所需 API 纳入依赖决策，不能直接复制示例。Severity Medium，关联 CORE §6/16、CCD §3.1、IP §6.2 |
| Git CLI（TECH §6） | 最低版本尚未指定，restore/pathspec-from-file 等需能力矩阵；无法仅用“系统 Git”承诺全部命令可用。Severity High：先确定最低支持版本及不支持时拒绝/降级策略 |

GPUI Editor/VirtualList/测试窗口的具体方法签名、生命周期和三平台截图路径尚未逐项编译验证，UI-001 必须覆盖；不将“文档提到组件”视为真实集成完成。

## 5. 审查处置

M0 先处理 AR-01/09/11/12 及 API 契约归属；AR-02–08 的安全边界、禁止组合和后续验证任务必须在 M0 定义，在相应写功能开放前完成。其余任务依赖缺口详见 M0-EXECUTION-CHECKLIST.md。当前建议批准文档收敛和受控技术验证，不批准把部分提交标为可直接生产实现。

