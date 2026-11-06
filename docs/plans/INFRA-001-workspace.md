# INFRA-001：Rust Workspace 骨架

依据 IP-001 §3.2、Git Core LLD §18.1、UI LLD §3 和已批准 ADR D1。
所有包继承 version 0.1.0、Edition 2024、publish = false；根采用 Resolver 3。
只有本地路径依赖，源码仅含职责文档和空 desktop 入口，没有业务 API、Git 调用或真实 UI。

## 成员与直接编译依赖

箭头方向为当前包依赖右侧包；Desktop 是具体实现装配根，UI 不依赖适配器。

| 路径 | 直接依赖 |
| --- | --- |
| `crates/core/git-domain` | 无 |
| `crates/core/git-ports` | `git-domain` |
| `crates/core/git-diff` | `git-domain` |
| `crates/core/git-graph` | `git-domain` |
| `crates/core/git-changelist` | `git-domain` |
| `crates/core/git-commit` | `git-ports`, `git-diff` |
| `crates/core/git-operations` | `git-ports`, `git-commit` |
| `crates/core/git-application` | `git-operations`, `git-ports` |
| `crates/adapters/git-cli` | `git-ports`, `git-domain` |
| `crates/adapters/git-libgit2` | `git-ports`, `git-domain` |
| `crates/adapters/repository-watcher` | `git-ports`, `git-domain` |
| `crates/adapters/credential-broker` | `git-ports`, `git-domain` |
| `crates/adapters/persistence` | `git-ports`, `git-domain` |
| `crates/ui/idea-theme` | 无 |
| `crates/ui/idea-ui-primitives` | `idea-theme` |
| `crates/ui/git-ui-model` | `git-application` |
| `crates/ui/idea-shell` | `idea-ui-primitives`, `git-ui-model` |
| `crates/ui/git-workbench` | `git-ui-model`, `idea-ui-primitives` |
| `crates/ui/git-commit-ui` | `git-ui-model`, `idea-ui-primitives` |
| `crates/ui/git-log-ui` | `git-ui-model`, `idea-ui-primitives` |
| `crates/ui/git-branch-ui` | `git-ui-model`, `idea-ui-primitives` |
| `crates/ui/git-diff-ui` | `git-ui-model`, `idea-ui-primitives` |
| `crates/ui/git-merge-ui` | `git-ui-model`, `idea-ui-primitives` |
| `crates/platform/desktop-platform` | 无 |
| `apps/desktop` | `git-application`, `git-cli`, `repository-watcher`, `credential-broker`, `persistence`, `desktop-platform`, `idea-shell`, `git-workbench`, `git-commit-ui`, `git-log-ui`, `git-branch-ui`, `git-diff-ui`, `git-merge-ui` |

## 范围与设计差异

- 以 IP 的完整目录为准，包含 ARCH 目录示例遗漏的 git-commit、idea-ui-primitives、git-ui-model；未改变已批准模块边界。
- git-libgit2 仅保留空包，没有第三方依赖，Desktop 未接入该可选适配器。
- 暂不创建未来功能子模块、资源、打包、CI、Git fixture 或运行时实现。
- 构建自动生成的 Cargo.lock 仅记录本地包，不代表 INFRA-002 的第三方依赖锁定已完成。
- rust-toolchain.toml 和第三方依赖核验留给 INFRA-002；不设置未经验证的 MSRV，不更改 Rust 1.99.1 候选基线。
- 当前无业务逻辑和失败分支，不添加占位断言；cargo test 用于验证全部测试目标和文档测试可构建，实际测试用例为 0。
