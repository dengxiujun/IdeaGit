# M0-EXECUTION-CHECKLIST：执行前置与里程碑依赖审查

日期：2026-11-06。状态：**待执行清单，未勾选项不代表完成**。本轮只审查；工程创建、API 修改及测试留给后续任务。文件简称和严重程度见 [ARCH-REVIEW-001](../reviews/ARCH-REVIEW-001.md)。

## 1. M0 必须先决定的事项

| 顺序 / 程度 | 关联文件 / 发现 | 原因 | 建议、责任与退出证据 |
|---|---|---|---|
| 1 High | CORE §2、IP §10；AR-09、API-12 | 编译依赖与类型所有权未冻结 | 架构负责人形成唯一 crate/DTO 所有权 ADR；GitCommand/Selection/ExecutionContext 不产生环；作为 INFRA-001 输入 |
| 2 Critical | CORE §8、CCD §9；AR-01–04 | Changelist 写入路径相互冲突，事务安全未证明 | Git Core 负责人确认 M2 仅 Index Commit；M0 定义高级功能禁用边界与验证计划，M4 开放前完成协议及故障测试 |
| 3 High | CORE §11、UI §6/9、IP §10；API-01–13、ST-01–05 | Mock/UI/Core 无共同契约 | 冻结首切片 Snapshot、Command、Result、事件和错误；记录高级接口扩展位、取消/未知结果/恢复的映射 |
| 4 High | TECH §1–8、IP §3；AR-12 | 候选版本存在但组合未构建 | Platform 负责人按官方 manifest 锁定候选；明确 Rust 日常版本/MSRV、Git 最低版本、三平台 target/SDK；INFRA-002 产生实际锁文件和验证报告 |
| 5 High | IP §3/5/6/7；下表 PLAN-* | 任务遗漏与安全前置不足 | 技术负责人批准具体任务依赖修订提案，补 Clone、Diff 前移、锁/Journal/Recovery 门禁；不擅改 IP 原文 |
| 6 Medium | UI §1/4/15、TECH §8 | Dock/Editor/虚拟化/截图能力需要真实验证 | UI-001 覆盖 Dock、Popup/焦点、IME、VirtualList、基础 Editor、Entity 调度和窗口关闭；保存各 OS 实测结果 |
| 7 High | IP TEST-001、CORE §13、CCD §16 | 高风险语义不能依赖 Mock | QA 建立隔离 Git 配置、hooks、signing、credentials 的临时仓库；本地 bare remote；保留外部竞争及故障注入能力 |
| 8 Medium | ARCH §17、IP §13；AR-16 | 多套 ADR/阶段编号冲突 | 统一 ADR 索引、主里程碑与 Core/UI/CCD 子阶段映射，后续变更走独立文档修订 |

建议拆分“决定候选版本/依赖边界”与“Workspace 创建后生成锁文件/构建验证”，避免 INFRA-001 必须先能 check、INFRA-002 才决定可用工具链的隐含倒置。INFRA-004 的架构决策部分应前置；PR 模板等可后置。

## 2. IP-001 M0～M6 依赖发现

仅按明确任务 ID 的已声明边检查，未见显式闭环；但“相关任务”、M2/M5、UI-M5 等不是完整 ID 邻接表，不能宣布任务图已机械验证无环。

| ID / 程度 | 关联文件 / 阶段 | 原因 | 建议 |
|---|---|---|---|
| PLAN-01 High | IP §3.1–3.4，M0 | INFRA-004 在 Workspace 后才制定方向；CI stable 与固定版本冲突；UI-001 未显式依赖 CI 环境 | 设计决策前置；候选版本→最小 Workspace→锁定/三平台环境→UI Spike→冻结。实现任务保持独立 |
| PLAN-02 Medium | IP §4.1/4.3，M1 | CORE-002 Discovery 未声明 CLI Executor 前置；CORE-006/007 未明确发现位置依赖 | 纯解析可并行；Discovery 集成依赖 CORE-003，Session 集成显式依赖 CORE-002；Snapshot 要声明读取窗口一致性 |
| PLAN-03 Critical | IP §5.1–5.5，M2 | GIT-001/003 仅依赖 CORE-010；CORE-011 Lock 与 CORE-012 Journal 在其后，不能保证写入验收依赖二者 | 将 coordinator 接口与可执行门禁分开：001/003/002 写操作验收必须依赖 011/012、preflight/reconcile 和基础恢复；不得用无锁 stub 宣布完成 |
| PLAN-04 High | IP §6.1 RECOVERY-001，M2→M3 | M2 已可提交但启动恢复到 M3 才交付 | 前移 Commit/Index 未知结果恢复的最小闭环；M3 再扩展 Sequencer/远程恢复 |
| PLAN-05 High | ARCH §16、UI §17、CCD §17.1；IP §6.1 | 首条切片含 File/Unified Diff，但 DIFF-001/002 到 M3 | 将最小只读 Diff 前移 M1/M2 或明确缩减首切片并批准；高级 Patch AST 和行级操作仍留 M4 |
| PLAN-06 High | IP §1.1/6.7，M3 | Clone 为 M3 交付且门禁从 Clone 开始，却无对应任务；REMOTE-004 Pull 仅依赖 Fetch，缺 Merge/Rebase/恢复；Fetch 也需认证 | 增加 Clone 任务及路径/取消/清理验收；Pull 依赖 merge 或明确支持的 rebase 路径、credential 与 recovery；Fetch 联调依赖 REMOTE-002 |
| PLAN-07 Medium | IP §5/6，M2/M3 | UI-007 未显式依赖 Action Registry；UI-009 缺 Shell；TEST-002/003 依赖“相关任务” | 展开为实际任务 ID；UI-007→UI-006，UI-009→UI-003；写明每个 E2E 所用链路与 fixture |
| PLAN-08 Critical | IP §7.1，M4 | UI-013 仅依赖 CCD-003/006，可能先于 CCD-007 实际 Index 协调与 TEST-004 安全测试发布 | UI 可 Mock 并行，真实功能开放必须依赖 CCD-007、TEST-004、Recovery；CCD-004/006 补锁、Journal、标准 Commit、Patch Builder 前置 |
| PLAN-09 High | IP §7.1、CCD §11.6，M4 | 有三版本 Viewer，却无明确 EditableIndexSession/Apply、Preview 生命周期任务；DIFF-003 与 DIFF-001 都称 AST | 拆分展示 parser 与可写完整 Patch AST；增加编辑会话/过滤/校验/Apply 验收任务；Line Stage 不依赖显示行号投影 |
| PLAN-10 High | IP §8.1，M5 | REBASE-004 仅依赖 RECOVERY-001，未显式依赖执行器；SHELF-002 未写协调器/恢复；UI-015 名称含 Stash 却只依赖 Shelf | REBASE-004 加 REBASE-001/003 联调；Shelf 写入加安全执行依赖；UI-015 加 GIT-007，TEST-005 展开实际任务 ID |
| PLAN-11 High | IP §9.1，M6 | SECURITY 审查/QA 故障注入过晚；RELEASE 仅依赖 M5，不显式依赖 QA、安全门禁；QA-003 引用 UI-M5 非 IP 任务 | 基础安全测试随写功能前置，M6 做全量复核；发布候选依赖 QA/安全/迁移通过；拆分“打包可并行”和“可发布” |
| PLAN-12 High | ARCH §15、CORE §14、UI §16、CCD §17、IP §1 | 同名 M3 在 Core 是 Diff/Log、IP 是远程；ARCH M4 含 Shelf/Rebase，IP 在 M5；M0 UI/Dock 范围也不同 | IP 作为建议主排期，子系统阶段用命名空间映射；必须批准范围裁决，不能只按 M 数字交付 |

## 3. 建议执行顺序及核验清单

以下是后续执行建议，本次均未运行。

- [ ] 完成上表 1–5 的决策记录与 API 冲突裁决，明确未解决高级能力为禁用。
- [ ] INFRA-001：创建最小成员与已批准依赖方向，避免提前实现功能；验证成员继承 edition/version/rust-version。
- [ ] INFRA-002：确认工具链可安装，统一 GPUI 重导出；逐项核对 Tokio、tokio-util、async-trait、thiserror、Serde、UUID、notify、tracing、测试依赖的实际 patch/features/MSRV；可选 git2 后置。
- [ ] 生成并保留 Cargo.lock；使用 cargo metadata --locked --format-version 1 和 cargo tree 检查实际方向、重复 GPUI 类型及 feature 传播。
- [ ] INFRA-003：三平台固定工具链，记录 Windows MSVC/SDK、macOS/Xcode、Linux 原生库、图形会话/GPU 条件。
- [ ] UI-001：不仅 cargo check，还需真实窗口、输入、Dock、焦点、Editor、列表及任务关闭试验；记录失败与替代方案。
- [ ] UI-002：采用工程初值，同时建立指定 IDEA 小版本的可追踪参考；不把初值当像素一致证明。
- [ ] TEST-001：独立 fixture 中验证初始化、提交、冲突、本地 remote 和配置隔离；覆盖 Unborn、linked worktree、特殊路径。
- [ ] 执行 fmt/check/clippy/test（--locked；clippy --all-targets -- -D warnings），记录 OS、toolchain、Git、锁文件对应版本和输出。
- [ ] M0 退出：三平台构建及窗口证据、实际无环依赖图、fixture 可用、首切片 API 已冻结；阻断问题已解决或对应功能明确禁用，风险有负责人和目标里程碑。

## 4. 写功能开放前的最低安全矩阵

关联 AR-01–08/14–15（Critical/High）、CCD §16；原因是文档规则尚无运行证据。建议由 QA/Git Core 共同维护：

| 场景 | 必须得到的证据 |
|---|---|
| 原有 staged A，仅选 B，含重叠/抵消 | 实际 commit tree 只含确认内容，剩余 Index/worktree 内容与预期一致 |
| 校验后外部改 Index/HEAD，两个 linked worktree 并发 | 拒绝过期操作或明确恢复，不覆盖外部变更 |
| Hook 拒绝、修改候选、签名失败、提交后响应丢失 | 分辨未提交/已提交需恢复/未知结果，不重复 Commit |
| check/apply 之间竞争、进程被杀、Journal 截断 | 保留备份，按真实 Git 状态恢复；不把取消解释为回滚 |
| pathspec magic、非 UTF-8、CRLF/filter、binary、symlink、submodule | 不误选文件、不二次过滤；不支持组合安全拒绝 |
| Diff 截断/ignore-whitespace/textconv、Hunk 指纹过期 | 展示不可直接成为写入来源，过期选区需重新确认 |
| Changelist 迁移、空列表、多 worktree/多窗口、订阅 lag | 元数据不串仓库、不丢草稿，状态可重新同步 |

这些是未来验收要求，不是本轮测试通过声明。

