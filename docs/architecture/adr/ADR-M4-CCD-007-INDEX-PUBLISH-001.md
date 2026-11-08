# ADR-M4-CCD-007：真实 Index Reconciliation 发布协议

- 状态：Draft，待项目负责人批准
- 日期：2026-11-08
- 范围：M4 CCD-007 / CCD-008 / TEST-004 / UI-013

## 1. 问题背景

M4 已具备临时 Index、Candidate Tree、Partial Commit 状态机、提交后副作用观察和保守的
`RealIndexReconciler` 规划器，但尚未实现真实 Index 的安全比较与发布。当前
`PartialCommitCoordinator` 在 `CommitObserved` 后不能把剩余 staged 内容协调到新 HEAD，也不能
将发布失败持久化为可恢复状态。因此不能证明“提交成功但 Index 协调失败”不会覆盖外部修改。

## 2. 阻断原因

直接在 `PartialCommitCoordinator` 或 Git CLI 提交适配器中写 `.git/index` 会跨越现有端口边界，
并且容易把进程内锁误当成 Git/外部进程锁。直接复用普通 `IndexWriter` 也不能表达
“读取当前真实 Index、比较预期摘要、在发布前再次确认、失败时不覆盖”的协议。
该变更涉及核心写入边界、端口契约和恢复语义，必须先获得 ADR 批准。

## 3. 候选方案

### 方案一：在 `PartialCommitCoordinator` 内直接写真实 Index

协调器读取 Index 文件并执行比较、备份和替换。

### 方案二：新增真实 Index 发布端口，由 Git CLI 适配器实现

新增应用可调用的端口，例如 `IndexReconciliationPublisher`，接收：

- `RecoveryScope`/仓库目标；
- 原始真实 Index 摘要 `I0`；
- 已验证的候选 Index 字节 `I1`；
- 预期 HEAD/操作上下文。

端口实现必须在适配器内完成：读取当前 Index、比较 `I0`、获取适用的 Git Index 写锁或等价
安全发布机制、再次确认外部变化、原子发布 `I1`，并返回 Published、Conflict 或 Unknown。
协调器只根据结构化结果更新 `ReconcilingIndex`、`Completed` 或 `RecoveryRequired` Journal。

### 方案三：不发布真实 Index，仅将所有提交结果标记为恢复

保留真实 Index 不变，提交后始终要求用户手动恢复。

## 4. 方案对比

| 方案 | 优点 | 风险 |
|---|---|---|
| 一 | 实现路径短 | 违反端口边界，容易覆盖外部 Index，难以证明跨平台安全 |
| 二 | 责任边界清晰，可替换适配器并集中测试安全协议 | 需要新增端口、适配器实现、锁/原子发布和故障注入测试 |
| 三 | 不会自动覆盖 Index | 无法完成 M4 的 Partial Commit 语义，长期阻塞 UI-013 |

## 5. 推荐方案

推荐方案二。端口只允许“基于预期摘要的安全发布”，不允许无条件覆盖；协调器在提交成功后
先记录 `CommitObserved`，再进入 `ReconcilingIndex`，发布失败、比较冲突、锁异常或结果不确定
均写入 `RecoveryRequired`，保留实际 commit OID、Index 备份和诊断信息，不自动重试或恢复覆盖。

## 6. 影响范围

- `git-ports`：新增真实 Index reconciliation 发布契约及结构化结果；
- `git-commit`：接入发布端口，补齐 `ReconcilingIndex`、`Completed`、`RecoveryRequired` 转移；
- `git-cli`：实现真实仓库的摘要比较、锁内发布、外部并发拒绝和失败分类；
- `git-operations`/组合根：确保发布经过 `OperationCoordinator` 的资源锁和 Journal；
- TEST-004：增加发布成功、外部 Index 变化、锁失败、发布后读取不一致和重启恢复测试。

不更换 Git CLI、Rust 依赖或 Git 数据模型；在批准前不开放 UI-013，不将 CCD-007 或 TEST-004
标记为完成。

## 7. 验证标准

1. 无竞争时，真实 Git 仓库中只发布经过摘要校验的 `I1`，未选 Worktree/Index 内容保留。
2. 外部修改真实 Index、HEAD、Worktree 或 refs 时，发布被拒绝且真实 Index 不变。
3. 发布过程失败、锁冲突、发布后读取不一致时，Journal 为 `RecoveryRequired`，保留实际 commit OID。
4. 模拟进程在 `CommitObserved`/`ReconcilingIndex` 阶段退出后，启动检查能分类为
   `CommittedNeedsReconcile` 或 `Unknown`，不得自动重复提交或覆盖当前 Index。
5. 覆盖普通文件、Rename、CRLF、二进制、特殊路径、Hook、跨平台锁语义，并通过 workspace
   fmt/check/clippy/test 与 `git diff --check`。
