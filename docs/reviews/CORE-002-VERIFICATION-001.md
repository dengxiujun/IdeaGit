# CORE-002-VERIFICATION-001：Repository Discovery 验证记录

- 日期：2026-11-07
- 任务：CORE-002
- 当前状态：已完成；当前 macOS 检查及 Rust CI #11 三平台验证全部通过

## 1. 范围与统一契约

本任务实现 `RepositoryReader.discover(PathBuf, ExecutionContext) -> RepositoryLocation`，覆盖 ordinary repository、linked worktree、bare repository 与 submodule。不实现 RepositoryId/WorktreeId 分配、Session Registry、Status/HEAD/Snapshot、Watcher、写操作或 UI。

`RepositoryLocation` 位于 `git-ports`，字段语义严格采用 Git Core §18.2：

- `worktree_root: Option<PathBuf>`：bare 为 `None`，不使用空路径冒充。
- `git_dir`：当前 worktree 自己的 Git dir。
- `common_dir`：linked worktree 共享的 common Git dir。
- `is_bare`：与 worktree 是否存在保持构造级一致。

未新增另一套仓库身份、类型或 capability DTO；RepositoryId/WorktreeId 与 capability 由后续 Session/Reader 任务建立。

## 2. 发现与路径安全

生产实现 `GitCliRepositoryReader` 通过 sealed 强类型命令分别调用：

```text
git -c core.quotepath=false rev-parse --is-bare-repository
git -c core.quotepath=false rev-parse --git-dir
git -c core.quotepath=false rev-parse --git-common-dir
git -c core.quotepath=false rev-parse --show-toplevel  # 仅非 bare
```

工作目录与参数以 OS 原生类型传入，不经 Shell。实现不拼接或猜测 `.git`：相对输出只相对于已规范化的候选目录解析，随后对 Git 报告的 worktree/git/common 路径执行真实文件系统 canonicalize。因此 linked worktree、submodule、nested repository 和符号链接解析都以 Git 与文件系统结果为准。

每次查询设置 `GIT_OPTIONAL_LOCKS=0` 和 `GIT_TERMINAL_PROMPT=0`；composition root 可注入显式环境及 Git executable。集成测试把 TEST-001 的 HOME、USERPROFILE、全局/系统配置和终端提示隔离环境传给生产 reader，不读取用户全局 Git 配置。

路径输出按原始字节处理：只移除 Git 为单条记录追加的一个 LF/CRLF。Unix 使用 `OsStringExt` 保留非 UTF-8；Windows 将 Git for Windows 的输出校验为 UTF-8 后构造原生 PathBuf，非法编码返回 `UnsupportedEncoding`。

## 3. 当前平台实际验证

当前环境：macOS 15.8 arm64、Rust/Cargo 1.99.1、Apple Git 2.39.5。

新增测试覆盖：

1. 从普通仓库的嵌套目录发现正确 worktree/git/common 路径。
2. primary 与 linked worktree 的 worktree/git_dir 不同而 common_dir 相同。
3. 本地 bare remote 返回 `worktree_root=None`，git_dir/common_dir 指向同一 bare 路径。
4. 本地 file-protocol submodule 被发现为自己的 worktree 与 `.git/modules/...` Git dir，不误归属 superproject。
5. 普通目录、缺失目录和文件路径分别返回结构化 `RepositoryNotFound`/`InvalidRepository`。
6. Unix 原始路径只去掉末尾记录终止符，保留路径内部换行。
7. `RepositoryLocation` 构造器不允许 bare 暴露伪 worktree。

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo check -p git-ports -p git-cli --all-targets --locked` | 通过 |
| `cargo clippy -p git-ports -p git-cli --all-targets --locked -- -D warnings` | 通过 |
| `cargo test -p git-ports -p git-cli --locked` | 通过；21 个测试全部成功 |
| `cargo check --workspace --all-targets --locked` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 |
| `cargo test --workspace --locked` | 通过；47 个 Workspace 测试全部成功 |
| `RUSTDOCFLAGS='-D warnings' cargo doc -p git-ports -p git-cli --no-deps --locked` | 通过；公共 API 文档无警告 |

## 4. 三平台验证

[Rust CI #11](https://github.com/dengxiujun/IdeaGit/actions/runs/42313168974) 对提交 `c97124c` 执行固定 Rust 1.99.1 的 locked metadata、check、Clippy、Workspace tests 和窗口烟雾测试，结果全部成功。ordinary nested path、linked worktree、bare remote 与本地 submodule 的真实 Git 发现测试在三个系统上实际执行。

| Job | 结果 | 证据 |
| --- | --- | --- |
| Format | 通过 | [Job](https://github.com/dengxiujun/IdeaGit/actions/runs/42313168974/job/128219293470) |
| Windows Server 2022 | 通过 | [Job](https://github.com/dengxiujun/IdeaGit/actions/runs/42313168974/job/128219293129) |
| macOS 15 | 通过 | [Job](https://github.com/dengxiujun/IdeaGit/actions/runs/42313168974/job/128219293636) |
| Ubuntu 24.04 | 通过 | [Job](https://github.com/dengxiujun/IdeaGit/actions/runs/42313168974/job/128219293446) |

## 5. 依赖、设计差异与遗留事项

- 无新增第三方依赖或版本变化；`git-fixture` 仅作为 `git-cli` dev-dependency，并新增可复用的隔离环境与本地 submodule helper。
- 权威契约没有 `RepositoryKind::Submodule`；本轮不擅自增加类型。Submodule 验收以 Git 返回独立 worktree/git_dir/common_dir 为准。
- discovery 的多个只读查询之间外部进程仍可能移动或删除仓库；任何不一致会在 canonicalize 或后续命令中失败，不发布猜测位置。Session 级复用、身份与刷新由 CORE-007/008 负责。
- macOS 路径内换行解析已有字节级测试；ordinary、linked、bare、submodule 及 Windows/Linux 原生路径行为已由 CI #11 实际执行。符号链接身份、大小写敏感差异和非 UTF-8 仓库根仍留给后续平台矩阵。
