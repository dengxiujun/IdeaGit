# M2 Operation Feedback Verification 001

`idea-shell` 新增统一 `OperationFeedback`：Running、Succeeded、Failed。`WorkbenchController::set_operation_feedback` 可在 UI 线程更新窗口反馈区域，后续写操作 Action 可复用，不会将失败误显示为成功。

已通过：

```text
cargo check -p idea-shell --locked
cargo clippy -p idea-shell --all-targets --locked -- -D warnings
cargo test -p idea-shell --locked
```
