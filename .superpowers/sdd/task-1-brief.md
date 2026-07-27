## Task 1: 补充 engine 诊断日志字段 + 删除示例 println

**Files:**
- Modify: `src/crawl/engine.rs` (L173, L254-L267)
- Modify: `examples/novel_crawler.rs` (L60-L105, L144, L184-L185)

**Interfaces:**
- Consumes: 现有 `Response::title()` (浏览器模式)、`Response::body.len()`、`Response::request.callback`
- Produces: 更详细的 `tracing::info!` 日志（无需用户手写 println）

- [ ] **Step 1: 修改 engine.rs 的 `process_response` 入口 instrument 字段**

修改 [src/crawl/engine.rs#L173](file:///home/weng/wisp/src/crawl/engine.rs#L173)：

```rust
#[tracing::instrument(level = "trace", skip(ctx, resp), fields(status = resp.status, body_len = resp.body.len(), callback = ?resp.request.callback))]
pub(crate) async fn process_response(ctx: &EngineContext, resp: Response) {
```

- [ ] **Step 2: 修改 info/warn 日志，补充 title 字段**

修改 [src/crawl/engine.rs#L250-L267](file:///home/weng/wisp/src/crawl/engine.rs#L250-L267) 区块。先在 `let page_url_for_log = resp.url.clone();` 后新增一行：

```rust
let page_url_for_log = resp.url.clone();
let status_for_log = resp.status;
let title_for_log = resp.title().unwrap_or_else(|| {
    // HTTP 模式下 title 字段为 None，从已解析的 doc 取（若 spider 未 parse 则这里不取）
    None
}).unwrap_or_default();
let (items, follows) = spider.handle(resp).await;
```

然后修改 if/else 分支：

```rust
if items.is_empty() && follows.is_empty() {
    tracing::warn!(
        "handle 返回空 (items=0, follows=0): url={}, status={}, title={:?}",
        sanitize_url(&page_url_for_log),
        status_for_log,
        title_for_log,
    );
} else {
    tracing::info!(
        "handle 完成: url={}, status={}, title={:?}, items={}, follows={}",
        sanitize_url(&page_url_for_log),
        status_for_log,
        title_for_log,
        items.len(),
        follows.len(),
    );
}
```

注意：`title_for_log` 只在浏览器模式下有值；HTTP 模式下为空字符串。这样既不破坏 `Response::parse()` 的单次调用约束（不在此处调用 `parse`），又能在浏览器模式下提供有用信息。

- [ ] **Step 3: 运行测试验证不破坏现有逻辑**

Run: `cargo test --package wisp --lib crawl::engine`
Expected: PASS（所有现有 engine 测试通过）

- [ ] **Step 4: 删除示例 novel_crawler.rs 中的所有诊断 println/eprintln**

在 [examples/novel_crawler.rs](file:///home/weng/wisp/examples/novel_crawler.rs) 删除以下行：
- L60-L61: `let title = doc.select_one("title")...` + `println!("[首页]...")`
- L98-L105: `if follows.is_empty() { ... } else { ... }` 整个诊断块
- L144: `println!("[详情]...")`
- L184-L185: `println!("[章节]...")`

保留所有业务逻辑（选择器、follow 构造、NovelItem 组装）。

- [ ] **Step 5: 编译验证示例**

Run: `cargo build --example novel_crawler`
Expected: PASS（无编译错误，无 warning）

- [ ] **Step 6: 运行完整测试套件**

Run: `cargo test --workspace`
Expected: PASS（全绿）

- [ ] **Step 7: 提交**

```bash
git add src/crawl/engine.rs examples/novel_crawler.rs
git commit -m "engine: 补充 process_response 日志字段并移除示例手写诊断 println"
```

---

