# PERF-001 大仓库 Benchmark 基线

日期：2026-11-09（Asia/Shanghai）
分支：`feat/PERF-001`
测试入口：`cargo test -p git-cli --test m6_perf --locked -- --ignored --nocapture`

## 数据集与环境

- 仓库类型：普通 Git worktree，临时文件系统，单个文件树，100,000 个线性提交。
- Git：`git version 2.39.5 (Apple Git-154)`。
- 操作系统：macOS，`aarch64`。
- CPU：`10` 个逻辑并行度。
- 磁盘：测试夹具所在临时文件系统；未记录具体物理介质型号。
- 数据集生成：691ms，使用单个 `git fast-import` 流；不是被测 Status/Log 查询时间。
- 样本数：11 次首轮样本和 11 次后续样本。

## 实测结果

| 场景 | 首轮 P50 | 首轮 P95 | 后续轮次 P50 | 后续轮次 P95 |
|---|---:|---:|---:|---:|
| Status | 8.452ms | 9.970ms | 8.079ms | 8.274ms |
| Log 首页（limit=100） | 9.197ms | 9.571ms | 8.673ms | 9.088ms |

这些结果来自生产 `GitCliStatusReader` 和 `GitCliHistoryReader`，通过真实 Git CLI 读取隔离仓库。首轮不是严格冷缓存：测试没有清空操作系统文件缓存，因此不能将其解释为冷缓存结果。

峰值内存本次未采集。取消的 Status 请求已在真实 Git reader 上验证，当前 macOS 样本为 0.078ms；Changes Tree 和 Branch Popup 的 1,000 条纯 UI projection 基线见下节。大型 Diff、多仓库切换尚未有可调用的 benchmark 入口，本记录不将它们标记为通过。

## UI projection 与取消补充

- 数据集：1,000 个 Changes Tree 文件条目、1,000 个 Branch Popup 行，21 次样本。
- 环境：macOS/aarch64。
- Changes Tree：P50 0.091ms，P95 0.427ms。
- Branch Popup：P50 0.301ms，P95 1.030ms。
- 取消 Status：0.078ms 返回结构化 `Cancelled` 错误。
- UI projection 和取消结果不包含 GPUI 实际渲染帧率，也不包含峰值内存。

## 验收状态

- [x] 真实 Git 10 万 Commit 数据集可重复生成。
- [x] Status / Log 的 P50/P95 已记录。
- [x] Git、OS、架构、并行度、磁盘和样本信息已记录。
- [x] 1,000 文件 Changes Tree 与 1,000 行 Branch Popup projection 已记录。
- [x] 真实 Git Status 取消路径已记录。
- [ ] 严格冷缓存测量。
- [ ] 峰值内存测量。
- [ ] 大型 Diff、多仓库切换覆盖。
- [ ] 三平台基线。

因此，本文件是 PERF-001 的 macOS 基线证据，不代表 PERF-001 或 M6 门禁完成。
