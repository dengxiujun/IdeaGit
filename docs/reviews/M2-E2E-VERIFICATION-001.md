# M2 E2E Verification 001

## 已验证

真实 `git-fixture` 串联完成：

1. 写入并 Stage 文件；
2. 提交 staged Index 并读取实际 OID/父提交；
3. 创建 `feature/demo` 分支并切回 `main`；
4. 分页读取 Log，确认新提交可见。

执行命令：

```text
cargo test -p git-cli --test m2_e2e --locked
```

该测试证明 Git Core 适配器闭环；M2 §5.9 要求的 GUI Open → Status → Stage/Unstage → Branch → Commit → Log 仍需 Desktop Action/UI wiring 和失败态交互测试后才能关闭。
