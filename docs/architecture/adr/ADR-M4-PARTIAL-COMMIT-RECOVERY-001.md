# ADR-M4-PARTIAL-COMMIT-RECOVERY-001：Hook 副作用与崩溃恢复观察边界

- 状态：Draft，待项目负责人批准
- 日期：2026-11-08
- 范围：M4 TEST-004 / CCD-006 / CCD-007

## 1. 问题背景

TEST-004 已通过真实临时 Index 验证未选内容、Hook 拒绝、签名失败、重命名、CRLF、二进制、非 UTF-8 路径和单行 Patch 的多个安全场景。但 Git Hook 可能在提交成功前后修改 Worktree 或其他 Refs；当前 `CommitWriter` 只返回提交结果，`PartialCommitCoordinator` 没有统一的提交前后 Worktree/Refs 观察端口，无法证明副作用已被检测并进入 RecoveryRequired。

## 2. 阻断原因

若直接在 `GitCliCommitWriter` 中读取并比较 Worktree 状态，将把协调、快照和恢复职责下沉到适配器，无法覆盖非 CLI 适配器，也无法保证与 OperationCoordinator 的锁、Journal 和外部并发观察一致。若忽略副作用，Hook 修改用户文件可能被错误报告为安全成功。

## 3. 候选方案

1. 在 CommitWriter 内部自行快照并比较 Worktree/Refs。
2. 新增协调器使用的 `CommitSideEffectObserver` 端口：提交前记录受控观察快照，提交后重新读取 HEAD、Index、Refs 和 Worktree 指纹；不一致时写入 Journal 并返回 `CommittedButNeedsRecovery`/`RecoveryRequired`。
3. 仅增加测试，不实现运行时观察。

## 4. 方案对比

| 方案 | 优点 | 风险 |
|---|---|---|
| 1 | 改动集中 | 适配器职责越界，无法统一锁与恢复语义 |
| 2 | 符合 UI→应用→端口→适配器方向，可测试并支持多后端 | 需要新增端口、组合根装配及 Journal 结果映射 |
| 3 | 无架构改动 | 仍会错误报告 Hook 副作用，不能满足 M4 发布阻断条件 |

## 5. 推荐方案

推荐方案 2。观察端口只提供快照和比较所需的结构化事实，不直接写 Index；协调器在 CommitObserved 后先核对提交身份，再执行 Worktree/Refs 副作用观察。任何无法确定的结果进入 RecoveryRequired，不自动重试。

## 6. 影响范围

涉及 `git-ports` 观察端口、`git-commit` 协调器/Journaling、Git CLI 适配器实现、真实 Git fixture，以及 TEST-004 的 Hook 修改 Worktree、外部 Ref 竞争和崩溃重启测试。不会更换技术栈或改变 Git 数据模型。

## 7. 验证标准

- Hook 修改 Worktree：提交结果不虚报为已协调成功，Journal 明确进入 RecoveryRequired。
- Hook 修改 Ref/HEAD 或外部并发：保留实际提交身份，不覆盖外部变化。
- 进程在 CommitObserved/ReconcilingIndex 阶段退出：重启可诊断且不可重复提交。
- 正常提交、Hook 拒绝、签名失败和无副作用路径均通过真实 Git fixture。
- 通过 workspace fmt/check/clippy/test 与 `git diff --check`。

未获批准前，不实施上述架构变更，也不开放 UI-013 或将 TEST-004 标记为完成。
