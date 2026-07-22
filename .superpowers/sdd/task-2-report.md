# Task 2 报告：SitemapSpider 迁移为 SpiderBuilder::sitemap() 预设

## Status
DONE

## Commits
- 343f337 feat(builder): SpiderBuilder::sitemap() 预设替代 templates.rs

## 变更摘要
1. **`src/crawl/builder.rs`** — 在 `impl SpiderBuilder` 中新增 `sitemap(name, sitemap_urls, content_label)` 预设方法。复用 `on("default", ...)` 注册入口 handler：解析 sitemap.xml，正则提取 `<loc>` URL，follow 到 `content_label` 指定的 handler。
2. **`src/crawl/templates.rs`** — 整文件删除（103 行死代码：CrawlSpider / SitemapSpider / CrawlRule）。
3. **`src/crawl/mod.rs`** — 删除 `pub mod templates;` 声明（无 re-export 需清理）。
4. **`tests/sitemap_test.rs`** — 新建，2 个测试覆盖构建与解析。

## 实现细节
- 复用现有 `SpiderBuilder::on(label, handler)` API + `ClosureSpider` HashMap 路由，无需新增 trait/struct。
- `regex::Regex` 为 `Cargo.toml` 已有依赖（`regex = "1"`），直接用全路径 `regex::Regex::new(...)`，无需新增 import。
- `SpiderResponse::text()` 已存在（返回 `Result<String, WispError>`），用 `unwrap_or_default()` 兜底空字符串。
- 闭包模式 `move |resp| { let label = label.clone(); async move {...} }` 保证 `Fn` 可多次调用（外层捕获 `label`，内层 clone 给每次调用）。

## 测试摘要
- `cargo build --lib`：✅ 编译通过（7 个 pre-existing warnings，与本次改动无关）
- `cargo test --test sitemap_test -- --nocapture`：✅ 2 passed / 0 failed
  - `test_sitemap_builder_creates_spider ... ok`
  - `test_sitemap_parses_loc_urls ... ok`（验证从 sitemap XML 提取 2 个 `<loc>` URL，且 callback 正确设为 `"content"`）
- `cargo test --lib crawl::builder`：✅ 8 passed / 0 failed（确认未破坏现有 builder 测试）

## Concerns
无。templates.rs 经全项目搜索确认仅 mod.rs 引用 + 自身自引用，是纯死代码；迁移后功能等价于原 SitemapSpider（正则提取 `<loc>`），并通过 `with_callback(content_label)` 增强 callback 路由能力（原版无 callback）。
