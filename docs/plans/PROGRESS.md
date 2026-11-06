# IdeaGit 开发进度

更新日期：2026-11-06。

## 当前状态

处于架构收敛阶段；尚未创建 Cargo Workspace，M0 工程任务未完成。

## 已完成的文档工作

- 首轮审查：ARCH-REVIEW-001、API-CONFLICTS-001、M0-EXECUTION-CHECKLIST。
- 项目负责人已在聊天批准 [ADR-ARCH-REMEDIATION-001](../architecture/adr/ADR-ARCH-REMEDIATION-001.md) D1–D7，并要求先迁移 ADR 目录。
- 已将 docs/adr 移至 docs/architecture/adr，修正链接并记录 Accepted 状态。
- 根据 ADR 修订六份设计/基线/主计划：共享契约集中 Core §18；Index 写后恢复捷径移除；状态/恢复统一；任务前置和发布门禁补齐。
- M0 清单保留首轮发现，并补充设计修订与待验证事项的区分。

## 验证与限制

本次检查结果：

- 相对 Markdown 文件链接检查通过（架构文档及主计划/M0 清单/进度记录）。
- 从修订后的 IP 实际文件提取 90 个任务：无重复 ID、缺失依赖引用或循环。
- git diff --check 通过；旧 docs/adr 目录已不存在，新目录包含正式 ADR。
- 已清理旧 SnapshotVersion、冲突共享 DTO 定义、真实 Index 写后恢复捷径及仅按 RepositoryId 定位的 UI 执行上下文。

未执行 Cargo 构建、Rust 测试、依赖安装、三平台窗口或真实 Git 故障注入，因为没有工程且本次批准范围为文档修订。

## 未关闭的工程门禁

| 范围 | 状态 / 放行条件 |
|---|---|
| Rust/GPUI/其他依赖 | 候选保持；INFRA-002 实际解析/锁定，MSRV 和 Git 最低版本/能力矩阵待验证 |
| Workspace | 设计依赖已统一；实际 Cargo 图尚不存在，不能宣称已编译无环 |
| 标准写操作 | Lock/Journal/RECOVERY-000/真实 Git fixture 完成后才能开放 |
| Partial Commit / Index 编辑 | capability 默认关闭；锁内比较/发布、剩余 staged 转移、Hook/HEAD 竞争及 TEST-004 未验证 |
| UI | 多 worktree、分组键、草稿/订阅语义已写入设计；窗口/线程调度/IME/渲染未验证 |
| 发布 | M0–M6 均不能因文档完成而标记交付；最终仍需 QA、安全、迁移与阻断项清零 |

## 下一步

后续领取 INFRA-001 前复核已批准契约及 INFRA-004 决策输入；按单任务建立最小 Workspace。任何新的核心版本/边界变更先走 ADR。当前不自动开始工程创建。
