# Engine item 交付路径统一设计

> 日期：2026-08-03
> 状态：已实现

## 目标

把 Engine 内部 item 交付收敛到 `CrawlEvent::Item` 这一条 seam：`run_inner_many` 不再持有 items 收集器，`run/run_many` 保持公开签名和 typed `Result`，但改为事件流之上的便利适配器；流式路径不再积累完整 items。

## 背景与范围

当前存在三条 item 交付路径：

1. `ItemPipeline` 处理 item（转换/过滤/落盘）。
2. `CrawlStream` / 事件总线发送 `CrawlEvent::Item`。
3. `run/run_many` 把全部 item 收进 `Vec<Value>` 返回。

三条路径共用同一个内部 run 循环，但 `run_inner_many` 的接口为了 `run_many` 强制要求一个 items 收集器；`run_stream_driver` 被迫创建并填充一个从不读取的 Vec，流式爬取内存随 item 数无界增长。同时 `run_many` 依赖 typed `Result`，不能直接降级成流式事件里的字符串错误。

## 已确认决策

- 采用方向 A：事件驱动内部，保留 `run/run_many` 便利层。
- `run_inner_many` 删除 items 参数与内部收集；item 只经 `CrawlEvent::Item` 交付。
- `run_many` 消费内部事件流收集 items，并通过内部 typed outcome channel 保留 `WispError`。
- 公开 API 与 `CrawlEvent` 枚举不变；流式事件行为不变。
- 是否最终移除 `run/run_many` 的 items 返回值，留作后续独立决策，不在本次实现。

## 实现设计

### 1. 删除 EngineState 的 items 收集器

- `EngineRunDraft.items` 与 `EngineState.items` 字段删除（`crates/crawl/src/engine/context.rs`）。
- `run_inner_many` 签名去掉 `items` 参数（`crates/crawl/src/engine/lifecycle.rs`）。
- `process_page_items` 不再 push 到 `ctx.state.items`，只发 `CrawlEvent::Item(processed)`（`crates/crawl/src/engine/response/emit.rs`）。

### 2. typed outcome channel

`run_stream_driver` 增加 `outcome: tokio::sync::oneshot::Sender<Result<Vec<CrawlStats>>>`：

- 成功：先发 `CrawlEvent::DoneMany(stats)`，再 `outcome.send(Ok(stats))`。
- 失败：保持现有 `CrawlEvent::Error { url: "*" }` + `DoneMany(Vec::new())`，再 `outcome.send(Err(e))`。

流式公开行为不变，`run_many` 获得 typed 错误。

### 3. 内部流式入口

`Engine::run_stream_many` 的内部逻辑提取为：

```rust
pub(crate) fn run_stream_many_inner<S: Spider + 'static>(
    &self,
    spiders: Vec<S>,
) -> (CrawlStream, oneshot::Receiver<Result<Vec<CrawlStats>>>)
```

公开 `run_stream_many` 只返回 `.0`，不关心 outcome。

### 4. run_many 变为事件流适配器

```rust
pub async fn run_many<S: Spider + 'static>(
    &self,
    spiders: Vec<S>,
) -> Result<(Vec<CrawlStats>, Vec<serde_json::Value>)> {
    let (stream, outcome) = self.run_stream_many_inner(spiders);
    let mut items = Vec::new();
    let mut events = stream.events();
    while let Some(event) = events.next().await {
        if let CrawlEvent::Item(value) = event {
            items.push(value);
        }
    }
    let stats = outcome.await??;
    Ok((stats, items))
}
```

`Engine::run` 继续调用 `run_many`，签名不变。

## 接口变更

- 公开 API：无变化。
- 内部变更：
  - `run_inner_many` 删除 `items` 参数。
  - `run_stream_driver` 增加 `outcome` 参数。
  - `EngineRunDraft.items`、`EngineState.items` 删除。
  - 新增 `run_stream_many_inner`。

## 错误处理

- `run_many` 通过 oneshot 拿到原始 `WispError`，保持 typed 语义。
- 事件流消费者看到的致命错误仍是 `Error { url: "*" }` + `DoneMany(Vec::new())`，与现状一致。
- 流式路径不再有被丢弃的 item Vec。

## 测试计划

- 更新 `engine/tests.rs` 的 `make_ctx_with`：移除 `items` 字段构造。
- 保留并验证：
  - `run_many` 返回的 items 与事件流一致。
  - `run` 的 typed 错误传播（现有 `run_inner_test.rs` 错误用例）。
  - `run_stream` 事件顺序与 `items()` 过滤不变。
- 流式不收集 items 由“EngineState 无 items 字段”保证，不新增内部状态断言。

## 验证命令

```bash
cargo fmt --all
cargo nextest run -p wisp-crawl --all-features
cargo test --doc
cargo check -p wisp --all-features
```

## 后续

实施并确认后，建议记录 ADR：Engine item 交付是单一 `CrawlEvent::Item` seam，`run/run_many` 是事件流之上的便利适配器；未来若要移除 items 返回值，基于该 seam 再做一次独立决策。
  - `CrawlStream` 内部 stream 标记 `+ Send`，保证 `run_many` future 可跨线程。
