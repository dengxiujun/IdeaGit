# CORE-001-VERIFICATION-001：领域 ID、OID、Path 与 Errors 验证记录

- 日期：2026-11-07
- 任务：CORE-001
- 当前状态：已完成；当前 macOS 检查及 Rust CI #9 三平台验证全部通过

## 1. 范围与依赖

本任务只修改 `git-domain` 的纯值对象，不实现 Repository Discovery、Git CLI 进程、Status/HEAD 读取、Snapshot 或 UI。生产依赖仍只有工作区已锁定的 `serde`、`thiserror` 和 `uuid`；测试复用既有 `serde_json`，没有新增或升级第三方版本。

`cargo tree -p git-domain --locked` 未出现 Tokio、GPUI、Git adapter 或其他内部 crate，保持 Domain 无 UI/IO 实现依赖。`serde_json` 只位于 dev-dependencies。

## 2. 已实现契约

### 强类型身份

- `RepositoryId`、`WorktreeId`、`OperationId` 是互不兼容的 UUID 新类型。
- `SessionTarget` 同时绑定 Repository、可选 Worktree 与 session generation；bare repository 不用空路径冒充 worktree。
- `SnapshotRevision` 是 Session 范围的单调应用世代，`checked_next` 不允许溢出回绕，也不宣称 Git 原子快照。

### ObjectId

- `ObjectFormat` 明确区分 SHA-1（20 字节/40 hex）与 SHA-256（32 字节/64 hex）。
- `ObjectId` 只接受完整摘要；解析接受大小写 hex，展示及 Serde 统一为小写全长 hex。
- 构造器拒绝算法/长度不匹配，解析器拒绝缩写、其他长度及非法 hex。

### RepoPath 与 LocalPath

- `RepoPath` 私有保存原始字节；Eq/Ord/Hash 基于原始字节，不经过 Unicode 或大小写归一化。
- 构造与反序列化执行相同校验，拒绝 NUL、绝对路径、空组件、`.`、`..` 和空路径。
- 空格、Tab、换行、冒号与 Unix 非 UTF-8 字节保持无损；仅提供明确标注为展示用途的 lossy 文本。
- `LocalPath` 独立保存 OS 原生 `PathBuf`；Domain 不提供 RepoPath ↔ LocalPath 转换，平台可表示性由后续 adapter 负责。

### 结构化错误

- `GitErrorKind` 与 Git Core §18.6 的 30 个稳定分类一致，并提供 snake_case 机器码。
- `GitError` 携带 message、可选脱敏 diagnostic、SessionTarget、OperationId 和 `RecoveryAction` 列表。
- 错误没有通用 retry 布尔值；是否继续、刷新或恢复必须依据实际操作结果和仓库状态。

## 3. 实际测试

当前环境：macOS 15.8 arm64、Rust/Cargo 1.99.1。

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo check --workspace --all-targets --locked` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 |
| `cargo test -p git-domain --locked` | 通过，12 个单元测试 |
| `cargo test --workspace --locked` | 通过；新增 12 个领域测试，既有测试亦通过 |
| `cargo tree -p git-domain --locked` | 通过；无 UI、Tokio 或内部 crate 依赖 |
| `RUSTDOCFLAGS='-D warnings' cargo doc -p git-domain --no-deps --locked` | 通过；公共 API 文档无警告 |

新增测试覆盖：ID 类型隔离/Serde、bare/worktree target、revision 溢出、SHA-1/SHA-256、大小写 hex、非法长度/算法/hex、特殊与非 UTF-8 RepoPath、NUL/绝对/穿越路径、反序列化防绕过、Unix 原生 LocalPath，以及结构化错误上下文和无默认恢复承诺。

## 4. 三平台验证

[Rust CI #9](https://github.com/dengxiujun/IdeaGit/actions/runs/42275839590) 对提交 `4841400` 执行固定 Rust 1.99.1 的 locked metadata、check、Clippy、Workspace tests 和窗口烟雾测试，结果全部成功：

| Job | 结果 | 证据 |
| --- | --- | --- |
| Format | 通过 | [Job](https://github.com/dengxiujun/IdeaGit/actions/runs/42275839590/job/128091818420) |
| Windows Server 2022 | 通过 | [Job](https://github.com/dengxiujun/IdeaGit/actions/runs/42275839590/job/128091818745) |
| macOS 15 | 通过 | [Job](https://github.com/dengxiujun/IdeaGit/actions/runs/42275839590/job/128091818961) |
| Ubuntu 24.04 | 通过 | [Job](https://github.com/dengxiujun/IdeaGit/actions/runs/42275839590/job/128091819048) |

## 5. 设计差异与遗留事项

- 文档未规定 ObjectId 的内存布局；实现使用带显式算法的固定 32 字节存储，外部语义仍严格遵守完整 SHA-1/SHA-256 契约，不需要 ADR。
- `RecoveryAction` 本轮只作为 Core §18.6 错误 DTO，并采用 IP-001 RECOVERY-001 已列出的稳定动作；不实现恢复扫描、授权或执行。
- RepositoryId 与规范化 common Git dir 的绑定、WorktreeId 分配和 session generation 生命周期由 CORE-002/CORE-007 负责，本轮不伪造发现逻辑。
- Unix 非 UTF-8 RepoPath 已作字节级单测；Windows 原生路径可表示性仍需后续 adapter 测试。纯领域类型与 Workspace 已由 CI #9 完成三平台复验。
