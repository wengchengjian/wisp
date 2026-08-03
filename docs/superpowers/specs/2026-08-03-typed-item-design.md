# Typed Item 设计（下一个候选阶段）

> 日期：2026-08-03
> 状态：已实现

## 目标

引入一等 `Item<T = serde_json::Value>` 领域类型，统一携带 payload、来源元数据和稳定 id；三条交付路径（ItemPipeline、CrawlEvent、run/run_many）全部升级为 `Item<Value>`，输出按用户确认序列化整个 Item。该设计建立在 ADR-0013 的单一 `CrawlEvent::Item` seam 之上。

## 背景与决策

- 当前 item 全程是 `serde_json::Value`，来源信息靠手工塞 payload（如 `on_content` 的 `"url"`）。
- 三条路径锁死 `Value`，导致泛型 `Item<T>` 难以贯穿；本次只在类型擦除后的 seam 使用 `Item<Value>`，Spider/Page 生产端不改。
- 已确认：
  - `Item<T = Value>`，字段：`value`、`source_url`、`spider`、`callback`、`id`。
  - `value` 私有，提供 `value()` / `into_value()` / `with_value()`，`with_value` 重算 id。
  - `id = sha256(source_url | callback | canonical_json)`，hex 字符串，`Item::new` 时算一次。
  - 引入 `sha2` 稳定哈希依赖。
  - `ItemPipeline`、`CrawlEvent::Item`、`run/run_many`、`CrawlStream::items()` 全部升级为 `Item<Value>`。
  - 内置 pipeline 与 MCP 输出序列化整个 Item（B 方案）。

## Item 形态

```rust
pub struct Item<T = serde_json::Value> {
    value: T,
    source_url: String,
    spider: String,
    callback: Option<String>,
    id: String,
}

impl Item<serde_json::Value> {
    pub fn new(value: Value, source_url: &str, spider: &str, callback: Option<&str>) -> Self;
    pub fn value(&self) -> &Value;
    pub fn into_value(self) -> Value;
    pub fn with_value(&mut self, value: Value); // 重算 id
    pub fn try_typed<T: DeserializeOwned>(&self) -> Result<T, ...>; // 借用反序列化
}
```

`Item<T>` 的泛型只用于用户侧 typed 视图，不进入 engine seam。

## 来源附着点

`process_page_items` 在 engine 内收到 `items: Vec<Value>` 时，已能拿到：

- `source_url`：响应 URL（`process_response` 传入）
- `spider`：`spider.name()`
- `callback`：`resp.request.callback`

engine 在交付前统一 `Item::new`，Spider trait、`Page::item`、`Spider::on_item` 保持 `Value` 不变。

## Seam 变更

- `ItemPipeline::process_item(&self, item: Item<Value>, ctx) -> Option<Item<Value>>`
- `CrawlEvent::Item(Item<Value>)`
- `run/run_many` 返回 `Vec<Item<Value>>`
- `CrawlStream::items()` 产出 `Item<Value>`
- 内置 pipeline：
  - JSON/JSONL/Output 写整个 Item。
  - `BatchItemPipeline` 闭包收 `Vec<Item<Value>>`。
  - `FilterFieldsPipeline` 操作 `item.value`。
- MCP `crawl_site` 输出整个 Item 的 JSONL；`run_many` 调用处改为 `item.into_value()` 或直接序列化 Item。
- `Items` 运行时输出助手新增 `from_items(Vec<Item<Value>>)` 适配，保留现有 `Value` 构造。

## 接口变更

- 新增：`wisp_crawl::Item`、`Item::try_typed`、`Items::from_items`。
- 变更：`ItemPipeline` trait、`CrawlEvent::Item`、`run/run_many` 返回类型、`CrawlStream::items()`、内置 pipeline 输入。
- 依赖：workspace 增加 `sha2 = "0.10"`，`wisp-crawl` 使用。

## 测试计划

- `Item::new` id 稳定：同输入跨实例一致，不同 value/source/callback 不同。
- `with_value` 重算 id。
- `try_typed` 借用反序列化，不消耗 `value`。
- pipeline 收 `Item<Value>` 后可读来源、可替换 value。
- `run_many` 返回 `Vec<Item<Value>>`，来源字段与事件一致。
- MCP `crawl_site` 输出包含来源字段（更新现有断言）。

## 验证命令

```bash
cargo fmt --all
cargo nextest run -p wisp-crawl --all-features
cargo nextest run -p wisp --all-features
cargo test --doc
cargo check -p wisp --all-features
```

## 后续

实现时若发现 `sha256` 对短 item 开销可感知，可改为惰性 id 或短哈希，但保持跨 run 稳定。
