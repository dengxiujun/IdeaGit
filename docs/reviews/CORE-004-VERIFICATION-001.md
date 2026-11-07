# CORE-004-VERIFICATION-001：Porcelain v2 Status Parser 验证记录

- 日期：2026-11-07
- 任务：CORE-004
- 当前状态：完成；当前 macOS 与三平台 CI 均通过

## 1. 范围与契约

本任务实现 `git status --porcelain=v2 -z --branch` 的字节级 parser、Status DTO、`StatusParser`/`StatusReader` ports 与真实 Git CLI reader。不实现独立 HEAD reader、Snapshot Builder、Session、Watcher、写操作或 UI。

格式依据 [Git 官方 git-status 文档](https://git-scm.com/docs/git-status#_porcelain_format_version_2)：

- `1` ordinary、`2` renamed/copied、`u` unmerged、`?` untracked、`!` ignored 全部支持。
- `2` 记录在 `-z` 模式下从下一个 NUL token 单独读取原路径，不按文本行或 TAB 猜测。
- 未识别 `#` header 按官方扩展规则忽略；已识别 header 重复则拒绝模糊输入。
- 所有路径保持原始字节并经 `RepoPath` 校验，不使用展示字符串还原。

## 2. 数据保真与错误模型

`git-domain` 新增 `StatusData`、`StatusEntry`、ordinary/renamed/unmerged DTO，以及以下强类型信息：

- Index/Worktree `ChangeKind` 双状态。
- HEAD/Index/Worktree 和 conflict stage 1/2/3 的 mode/OID。
- SHA-1/SHA-256 完整 ObjectId；全零 mode/OID 显式映射为 `None`。
- `N...` / `S<c><m><u>` submodule 位。
- Rename/Copy 类型与 0..=100 similarity score。
- 七种标准 unmerged XY 组合。
- `branch.oid/head/upstream/ab`，其中 `(initial)` 与 `(detached)` 保持显式状态，ref 名保存原始字节。

`StatusParseError` 区分 truncated、unknown record、invalid field 与 duplicate header，并记录字节 offset；parser 不静默接受缺 NUL、缺 rename source、非法 mode/OID/status/submodule/score 或越界 RepoPath。

生产 `GitCliStatusReader` 强制 porcelain v2、NUL、branch、rename detection 及明确的 untracked/ignored 选项，并使用 `--no-optional-locks` 避免后台读取仅为刷新 stat cache 而写 Index。非零退出码不被解析为状态；bare 因无 worktree 明确返回 `UnsupportedFeature`。

## 3. 当前平台实际验证

当前环境：macOS 15.8 arm64、Rust/Cargo 1.99.1、Apple Git 2.39.5。

合成 parser 测试覆盖：全部记录类型、未知 header、branch headers、SHA-1/SHA-256、unborn、Rename/Copy score、七种 conflict、submodule 位、空格/Tab/换行/非 UTF-8 路径、zero OID/mode，以及截断、未知 record、重复 header、非法字段和路径穿越。

真实隔离 Git Fixture 测试覆盖：

1. tracked unstaged 修改。
2. staged addition 与同文件 staged+unstaged 共存。
3. `git mv` rename 及 original path。
4. untracked/ignored 选项。
5. 真实三方 `UU` merge conflict 及三个 stage。
6. bare repository status 拒绝。

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo check -p git-domain -p git-ports -p git-cli --all-targets --locked` | 通过 |
| `cargo clippy -p git-domain -p git-ports -p git-cli --all-targets --locked -- -D warnings` | 通过 |
| `cargo test -p git-domain -p git-ports -p git-cli --locked` | 通过；43 个测试全部成功 |
| `cargo check --workspace --all-targets --locked` | 通过 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 通过 |
| `cargo test --workspace --locked` | 通过；57 个 Workspace 测试全部成功 |
| `RUSTDOCFLAGS='-D warnings' cargo doc -p git-domain -p git-ports -p git-cli --no-deps --locked` | 通过；公共 API 文档无警告 |

## 4. 三平台 CI 验证

[Rust CI #12](https://github.com/dengxiujun/IdeaGit/actions/runs/42315434816) 已完成并全部通过：

- [Windows Server 2022](https://github.com/dengxiujun/IdeaGit/actions/runs/42315434816/job/128226772727)
- [Ubuntu 24.04](https://github.com/dengxiujun/IdeaGit/actions/runs/42315434816/job/128226772856)
- [macOS 15](https://github.com/dengxiujun/IdeaGit/actions/runs/42315434816/job/128226773027)
- [Format](https://github.com/dengxiujun/IdeaGit/actions/runs/42315434816/job/128226773077)

各平台均使用 Rust 1.99.1 与提交的 Cargo.lock，执行 locked check、Clippy、Workspace tests 和窗口烟雾测试。由此关闭本任务对 Windows/Linux 的待验证项；超大仓库性能、sparse checkout、LFS 与嵌套 submodule status 仍未验证。

## 5. 依赖、设计差异与遗留事项

- 无新增第三方版本；`git-ports` 直接复用 Workspace 已锁定的 `thiserror` 定义结构化 parse error。
- Core §18 的 `HeadState` 仍由 CORE-005 负责；本轮 BranchStatus 只保留同一次 status 响应的可选 header，不生成 Session 级 HEAD 真相或重复 `HeadState`。
- 权威计划沿用 `StatusEntry::Renamed(RenamedChange)` 命名；该 DTO 用 `SimilarityKind` 明确区分 Rename 与 Copy，不丢失 C 记录。
- status parser 保留 Git 的未定义输出顺序；UI 分组和稳定排序属于 Snapshot/UI projection，不在 adapter 中改变语义。
- Windows、macOS、Linux 的真实 Fixture 已由 Rust CI #12 通过。未执行超大仓库性能、sparse checkout、LFS 或嵌套 submodule status。
