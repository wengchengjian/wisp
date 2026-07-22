# Task 3: 删除 parse_fn/async_parse_fn 冗余

**Files:**
- Modify: `src/crawl/builder.rs`
- Modify: `tests/builder_api_test.rs`
- Modify: `tests/callback_routing_test.rs`（如有 parse_fn 用法）

## Steps

1. 删除 ParseFn / AsyncParseFn 类型（如存在）
2. 删除 SpiderBuilder 的 parse_fn/async_parse_fn 字段和 parse()/parse_async() builder 方法
3. 删除 ClosureSpider 的 parse_fn/async_parse_fn 字段
4. 改造 ClosureSpider::parse() 为兜底空实现
5. 更新 build() 断言：要求至少一个 handler
6. 迁移现有测试：`.parse(|resp| {...})` → `.on("default", |resp| async move {...})`
7. Grep 搜索 `.parse(|` 和 `.parse_async(|` 在 src/ 和 tests/，全部迁移
8. 验证：cargo build --lib + 相关测试
9. 提交（多 -m 参数）

## 关键说明
先读取当前 `src/crawl/builder.rs` 确认：
- ParseFn/AsyncParseFn 类型是否存在（可能已被之前重构删除）
- parse_fn/async_parse_fn 字段是否存在
- parse()/parse_async() 方法是否存在
- ClosureSpider 结构体当前字段

如果某些已不存在，跳过对应步骤。重点是用 Grep 搜索 `.parse(|` 找到所有调用点，迁移到 `.on("default", |resp| async move {...})`。

## 背景
parse_fn/async_parse_fn 是 SpiderBuilder 的旧单闭包 API，已被 on(label, handler) 多 handler API 取代。不向后兼容，统一到 on() API。

## 测试迁移示例
旧：
```rust
.parse(|resp| { (vec![], vec![]) })
```
新：
```rust
.on("default", |resp| async move { (vec![], vec![]) })
```

## 验证
```
cargo build --lib
cargo test --lib crawl::builder -- --nocapture
cargo test --test builder_api_test -- --nocapture
cargo test --test callback_routing_test -- --nocapture
```

## 提交
```
git add src/crawl/builder.rs tests/builder_api_test.rs tests/callback_routing_test.rs
git commit -m "refactor(builder): 删除 parse_fn/async_parse_fn，统一到 on() API" -m "ParseFn/AsyncParseFn 类型删除" -m "所有测试迁移到 on(label, handler)"
```
