# docs/superpowers（历史文档归档）

> **状态：历史/已废弃**
>
> 本目录中的 briefs、plans、specs 是开发过程的阶段性设计文档，
> 记录了 2026-07 至 2026-07-31 的 crate 拆分与架构演进过程。
> 其中大量 API 名称（如 `SpiderRequest`、`Session`、`request_cache`）
> 和模块路径已在后续重构中删除或重命名，**请勿据此实现新功能**。

当前权威文档：

- 项目使用说明：仓库根目录 [README.md](../../README.md)
- 已知问题与剩余工作：[docs/known-issues.md](../known-issues.md)
- 性能基准：[docs/performance.md](../performance.md)
- 当前代码：以 `crates/` 下源码和测试为准

如需追溯某个设计决策，可阅读对应文件；但任何与当前代码不一致的
描述，一律以当前代码和测试为准。
