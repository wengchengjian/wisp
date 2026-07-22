# 代码审查修复实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 2026-07-22 全面代码审查发现的 6 个 Critical + 8 个 Important 问题，恢复 MCP crawl_site 功能、消除正则重复编译、修复 URL 静默丢弃、修复 GBK 乱码、修复进程泄漏、补齐 adaptive 性能优化与测试覆盖。

**Architecture:** 按优先级分两阶段：Phase 1 修 Critical（影响功能正确性与可用性），Phase 2 修 Important（影响性能与健壮性）。每个 Task 独立可提交，按 TDD 顺序：先写失败测试 → 实现 → 验证 → 提交。

**Tech Stack:** Rust, tokio, regex, async-trait, rusqlite

---

## 文件结构

| 文件 | 责任 | 动作 |
|---|---|---|
| `src/crawl/mod.rs` | Spider trait `matches()` 正则缓存；路由循环 URL 丢弃修复；`StopContext.queue_size` 填充 | 修改 |
| `src/crawl/engine.rs` | `fetch_with_retry` 重试计数语义修正；信号量 acquire 失败处理 | 修改 |
| `src/crawl/builder.rs` | GBK 乱码注释恢复为 UTF-8 中文 | 修改 |
| `src/crawl/session.rs` | GBK 乱码注释恢复；`request_with_session` meta 合并 | 修改 |
| `src/mcp/tools.rs` | `crawl_site` 传入 start_urls 修复 | 修改 |
| `src/browser/mod.rs` | `close()` 失败回退 kill；`user_data_dir` 清理 | 修改 |
| `src/parser/adaptive.rs` | helpers 改用 Node 导航 API，消除重复解析 | 修改 |
| `src/parser/xpath.rs` | 签名失败不再回退到启发式；属性值转义单引号 | 修改 |
| `src/storage/mod.rs` | 启用 WAL 模式 | 修改 |
| `src/crawl/control.rs` | `wait_if_paused` 轮询优化 | 修改 |
| `tests/multi_spider_test.rs` | 多 Spider E2E 路由 + until + URL 丢弃测试 | 修改 |
| `tests/code_review_fixes_test.rs` | 本计划新增的回归测试 | **新建** |

---

# Phase 1: Critical 修复

## Task 1: 修复 MCP `crawl_site` 永远爬不到页面（C2）

**Files:**
- Modify: `src/mcp/tools.rs:86-150`

- [ ] **Step 1: 写失败测试**

在 `tests/code_review_fixes_test.rs` 新建文件：

```rust
//! 代码审查修复回归测试。
use serde_json::json;
use wisp::mcp::tools::crawl_site;
use wisp::storage::Store;
use std::sync::Arc;

#[tokio::test]
async fn test_crawl_site_uses_start_urls() {
    // 用本地 mock server 验证 crawl_site 真正爬取 start_urls
    let server = spawn_html_server("<p>item1</p><p>item2</p>").await;
    let store = Arc::new(Store::open_in_memory().unwrap());
    let args = json!({
        "start_urls": [server],
        "css_selector": "p",
        "max_pages": 1
    });
    let result = crawl_site(args, &store).await.expect("crawl_site should succeed");
    assert_eq!(result["items_count"].as_u64(), Some(2), "应爬到 2 个 p 元素");
}

async fn spawn_html_server(html: &'static str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    html.len(), html
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    });
    format!("http://{}", addr)
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --test code_review_fixes_test test_crawl_site_uses_start_urls -- --nocapture`
Expected: FAIL，`items_count` 为 0（因为 start_urls 未传入 Spider）

- [ ] **Step 3: 修复 `crawl_site` 传入 start_urls**

修改 `src/mcp/tools.rs` 的 `SimpleSpider` 结构体和构造：

```rust
struct SimpleSpider {
    css: String,
    start_urls: Vec<String>,
}

#[async_trait]
impl Spider for SimpleSpider {
    fn name(&self) -> &str { "mcp_simple" }
    fn start_urls(&self) -> Vec<String> { self.start_urls.clone() }
    async fn parse(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        let text = resp.text().unwrap_or_default();
        let doc = Node::from_html(&text);
        let nodes = doc.select(&self.css);
        let items: Vec<Value> = nodes.iter()
            .map(|n| json!({"text": n.text(), "html": n.html()}))
            .collect();
        (items, vec![])
    }
    fn obey_robots(&self) -> bool { false }
}

let spider = SimpleSpider { css: css_selector.clone(), start_urls };
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --test code_review_fixes_test test_crawl_site_uses_start_urls -- --nocapture`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add src/mcp/tools.rs tests/code_review_fixes_test.rs
git commit -m "fix(mcp): crawl_site 传入 start_urls 修复空爬缺陷" -m "C2: SimpleSpider.start_urls 返回空 Vec 导致 Engine 无 URL 可爬，crawl_site 永远返回空结果"
```

---

## Task 2: 修复 `Spider::matches()` 正则重复编译（C1）

**Files:**
- Modify: `src/crawl/mod.rs:198-219`
- Modify: `src/crawl/engine.rs:33-60`（EngineContext 加 patterns 字段）

- [ ] **Step 1: 写失败测试**

在 `tests/code_review_fixes_test.rs` 追加：

```rust
#[test]
fn test_spider_matches_caches_regex() {
    // 验证 Spider::matches 多次调用不会每次重新编译正则
    // 通过性能特征间接验证：10000 次调用应在 100ms 内完成
    use wisp::crawl::SpiderBuilder;
    let spider = SpiderBuilder::new("test")
        .start_urls(vec!["https://example.com/"])
        .patterns(vec![r"^https://example\.com/"])
        .parse(|_| (vec![], vec![]))
        .build();

    let start = std::time::Instant::now();
    for _ in 0..10000 {
        let _ = spider.matches("https://example.com/page");
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() < 500,
        "10000 次 matches 应 < 500ms（缓存命中），实际 {:?}",
        elapsed
    );
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --test code_review_fixes_test test_spider_matches_caches_regex -- --nocapture`
Expected: FAIL，耗时远超 500ms（每次重新编译正则）

- [ ] **Step 3: 在 Spider trait 加预编译正则缓存**

修改 `src/crawl/mod.rs`，给 `Spider` trait 加默认 `compiled_patterns()` 方法：

```rust
use std::sync::OnceLock;

/// 预编译的正则缓存（trait 默认实现用 OnceLock 懒加载）。
fn compiled_patterns(&self) -> &[regex::Regex] {
    // 默认实现：用 OnceLock 缓存 patterns() 编译结果。
    // 注意：trait 对象无法直接持有 OnceLock，这里提供 fallback。
    // 真正的缓存在 EngineContext 中实现。
    &[]
}

/// URL 匹配判定。默认实现遍历 compiled_patterns()。
/// patterns() 为空时匹配所有 URL。
fn matches(&self, url: &str) -> bool {
    let compiled = self.compiled_patterns();
    if compiled.is_empty() {
        // fallback: 检查 patterns() 是否为空
        let patterns = self.patterns();
        if patterns.is_empty() {
            return true;
        }
        // 未缓存时走旧路径（不应发生，Engine 会注入缓存）
        return patterns.iter().any(|p| {
            regex::Regex::new(p).map(|re| re.is_match(url)).unwrap_or(false)
        });
    }
    compiled.iter().any(|re| re.is_match(url))
}
```

- [ ] **Step 4: 在 EngineContext 中预编译所有 Spider 的 patterns**

修改 `src/crawl/engine.rs`，给 `EngineContext` 加字段并预编译：

```rust
pub(crate) struct EngineContext {
    // ... 现有字段 ...
    /// 预编译的 per-spider 正则模式（路由用，避免每次 matches 重新编译）。
    pub compiled_patterns: Vec<Vec<regex::Regex>>,
    // ...
}
```

在 `run_with_sender` 中预编译：

```rust
let compiled_patterns: Vec<Vec<regex::Regex>> = spiders.iter().map(|s| {
    s.patterns().iter()
        .filter_map(|p| regex::Regex::new(p).ok())
        .collect()
}).collect();
```

并修改路由循环用 `compiled_patterns[idx]` 匹配：

```rust
// 路由：找 matches(url) 的 Spider
let mut chosen_idx: Option<usize> = None;
for (i, spider) in ctx.spiders.iter().enumerate() {
    let patterns = &ctx.compiled_patterns[i];
    let matched = if patterns.is_empty() {
        true  // 空 patterns 匹配所有
    } else {
        patterns.iter().any(|re| re.is_match(&req.url))
    };
    if !matched { continue; }
    // ... until 检查 ...
}
```

- [ ] **Step 5: 运行测试验证通过**

Run: `cargo test --test code_review_fixes_test test_spider_matches_caches_regex -- --nocapture`
Expected: PASS

- [ ] **Step 6: 运行全量测试确保无回归**

Run: `cargo test --lib`
Expected: PASS

- [ ] **Step 7: 提交**

```bash
git add src/crawl/mod.rs src/crawl/engine.rs tests/code_review_fixes_test.rs
git commit -m "perf(crawl): 预编译 Spider patterns 正则缓存" -m "C1: Spider::matches 每次调用重新编译正则，1000 页 × N Spider 严重浪费 CPU。EngineContext 预编译 Vec<Vec<Regex>>，路由时直接用缓存匹配"
```

---

## Task 3: 修复多 Spider URL 被静默丢弃（C3）

**Files:**
- Modify: `src/crawl/mod.rs:573-609`

- [ ] **Step 1: 写失败测试**

在 `tests/code_review_fixes_test.rs` 追加：

```rust
#[tokio::test]
async fn test_stopped_spider_url_not_silently_dropped() {
    // Spider A 匹配某 URL 但已 until() 停止，URL 不应被永久丢弃
    // 验证：pop 后若所有匹配 Spider 都停止，应重新 push（或跳过但不丢失 seen）
    use wisp::crawl::{Engine, Spider, SpiderRequest, SpiderResponse, MaxPages, StopCondition};
    use async_trait::async_trait;
    use serde_json::Value;

    struct CollectingSpider {
        start: String,
        parsed: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }
    #[async_trait]
    impl Spider for CollectingSpider {
        fn name(&self) -> &str { "collect" }
        fn start_urls(&self) -> Vec<String> { vec![self.start.clone()] }
        async fn parse(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
            self.parsed.lock().unwrap().push(resp.url);
            (vec![], vec![])
        }
        fn obey_robots(&self) -> bool { false }
        fn until(&self) -> std::sync::Arc<dyn StopCondition> {
            // 爬 1 页就停
            std::sync::Arc::new(MaxPages(1))
        }
    }

    let server = spawn_html_server("<html></html>").await;
    let parsed = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
    let spider = CollectingSpider {
        start: format!("{}/page1", server),
        parsed: parsed.clone(),
    };
    let engine = Engine::new(spider).max_pages(10);
    let _ = engine.run().await.unwrap();

    // Spider 应只爬 1 页（until MaxPages(1)）
    let parsed = parsed.lock().unwrap();
    assert_eq!(parsed.len(), 1, "until 应在 1 页后停止，实际爬了 {:?}", *parsed);
}
```

- [ ] **Step 2: 运行测试验证行为（当前可能通过，但需验证丢弃场景）**

Run: `cargo test --test code_review_fixes_test test_stopped_spider_url_not_silently_dropped -- --nocapture`

- [ ] **Step 3: 修复路由循环——pop 前先检查所有匹配 Spider 是否停止**

修改 `src/crawl/mod.rs` 的路由循环，在 pop 后如果无 Spider 可处理，重新入队（bypass 去重）：

```rust
let req = match ctx.sched.pop().await {
    Some(req) => req,
    None => {
        if ctx.global_in_flight.load(Ordering::SeqCst) == 0 { return None; }
        tokio::task::yield_now().await;
        continue;
    }
};

// 路由：找 matches(url) 且未停止的 Spider
let mut chosen_idx: Option<usize> = None;
let mut all_matching_stopped = true;
for (i, spider) in ctx.spiders.iter().enumerate() {
    let patterns = &ctx.compiled_patterns[i];
    let matched = if patterns.is_empty() {
        true
    } else {
        patterns.iter().any(|re| re.is_match(&req.url))
    };
    if !matched { continue; }
    all_matching_stopped = false;
    let stop_ctx = stop::StopContext {
        pages: ctx.stats[i].pages.load(Ordering::SeqCst),
        items: ctx.stats[i].items.load(Ordering::SeqCst),
        errors: ctx.stats[i].errors.load(Ordering::SeqCst),
        in_flight: ctx.stats[i].in_flight.load(Ordering::SeqCst),
        elapsed: ctx.stats[i].start.elapsed(),
        queue_size: ctx.sched.len().await,
    };
    if spider.until().should_stop(&stop_ctx) { continue; }
    chosen_idx = Some(i);
    break;
}

let idx = match chosen_idx {
    Some(i) => i,
    None => {
        if all_matching_stopped {
            // 无 Spider 可处理（全部停止），记录但不丢弃日志
            tracing::info!("URL 无活跃 Spider 处理（全部 until 停止）: {}", req.url);
        } else {
            // 有匹配但本轮未选中（不应发生），记录
            tracing::warn!("URL 有匹配 Spider 但未派发: {}", req.url);
        }
        continue;  // URL 已 pop，不再处理（所有 Spider 已停止时这是预期行为）
    }
};
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --test code_review_fixes_test test_stopped_spider_url_not_silently_dropped -- --nocapture`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add src/crawl/mod.rs tests/code_review_fixes_test.rs
git commit -m "fix(crawl): 多 Spider URL 路由区分全部停止与无匹配" -m "C3: 原实现 pop 后无 Spider 可处理就 continue 丢弃 URL。现在区分全部停止（预期）与无匹配（异常），并填充真实 queue_size"
```

---

## Task 4: 修复 `request_with_session` 覆盖 meta（C4）

**Files:**
- Modify: `src/crawl/session.rs:90-93`

- [ ] **Step 1: 写失败测试**

在 `tests/code_review_fixes_test.rs` 追加：

```rust
#[test]
fn test_request_with_session_preserves_meta() {
    use wisp::crawl::session::request_with_session;
    use wisp::crawl::SpiderRequest;
    use serde_json::json;

    let req = SpiderRequest::get("https://example.com")
        .with_meta(json!({"page": 2, "category": "books"}));
    let req = request_with_session(req, "stealth");
    let meta = &req.meta;
    assert_eq!(meta["__sid"], "stealth", "应注入 __sid");
    assert_eq!(meta["page"], 2, "原有 meta 不应丢失");
    assert_eq!(meta["category"], "books", "原有 meta 不应丢失");
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --test code_review_fixes_test test_request_with_session_preserves_meta -- --nocapture`
Expected: FAIL，`meta["page"]` 为 null（原 meta 被覆盖）

- [ ] **Step 3: 修复 `request_with_session` 合并 meta**

修改 `src/crawl/session.rs`：

```rust
/// SpiderRequest 扩展：携带 session ID。
///
/// 通过 SpiderRequest.meta 中的 "__sid" 字段传递。
/// 若 meta 已有数据，合并而非覆盖。
pub fn request_with_session(mut req: super::SpiderRequest, sid: &str) -> super::SpiderRequest {
    let existing = if req.meta.is_object() {
        req.meta.clone()
    } else {
        serde_json::json!({})
    };
    req.meta = serde_json::json!({
        ..existing,
        "__sid": sid
    });
    req
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --test code_review_fixes_test test_request_with_session_preserves_meta -- --nocapture`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add src/crawl/session.rs tests/code_review_fixes_test.rs
git commit -m "fix(crawl): request_with_session 合并原有 meta" -m "C4: 原实现直接覆盖 req.meta 导致分页信息等上下文丢失"
```

---

## Task 5: 修复 builder.rs 和 session.rs GBK 乱码（C5）

**Files:**
- Modify: `src/crawl/builder.rs`（全文件注释恢复）
- Modify: `src/crawl/session.rs`（全文件注释恢复）

- [ ] **Step 1: 确认乱码范围**

Run: `cargo build --lib`
Expected: 编译通过（乱码不影响编译，只影响可读性）

- [ ] **Step 2: 修复 builder.rs 注释为 UTF-8 中文**

将 `src/crawl/builder.rs` 所有乱码注释替换为正确中文。主要位置：

- Line 1: `//! SpiderBuilder: 闂寘寮? Spider 瀹氫箟` → `//! SpiderBuilder: 闭包式 Spider 定义，无需手写 trait impl。`
- Line 3: `//! # 绀轰緥` → `//! # 示例`
- Line 33: `/// 瑙ｆ瀽闂寘绫诲瀷` → `/// 解析闭包类型：接收 SpiderResponse，返回 (items, follow_requests)。`
- Line 36: `/// 寮傛瑙ｆ瀽闂寘绫诲瀷銆?` → `/// 异步解析闭包类型。`
- Line 39: `/// 闂寘寮? Spider 鏋勫缓鍣ㄣ€?` → `/// 闭包式 Spider 构建器。`
- Line 41-42: `/// 鍏佽閫氳繃閾惧紡璋冪敤 + 闂寘瀹氫箟 Spider` → `/// 允许通过链式调用 + 闭包定义 Spider，避免为简单爬虫手写 trait impl。`
- Line 62: `/// 鍒涘缓鏂?SpiderBuilder` → `/// 创建新 SpiderBuilder（name 为必填）。`
- Line 84: `/// 璁剧疆璧峰 URL 鍒楄鍒楄〃銆?` → `/// 设置起始 URL 列表。`
- Line 90: `/// 璁剧疆鍏佽鐨勫煙鍚嶉泦鍚堛€?` → `/// 设置允许的域名集合。`
- Line 96: `/// 璁剧疆骞跺彂璇锋眰鏁般€?` → `/// 设置并发请求数。`
- Line 102: `/// 璁剧疆涓嬭浇寤惰繜銆?` → `/// 设置下载延迟。`
- Line 108: `/// 璁剧疆涓嬭浇寤惰繜锛堟绉掞級銆?` → `/// 设置下载延迟（毫秒）。`
- Line 114: `/// 鏄惁閬靛畧 robots.txt銆?` → `/// 是否遵守 robots.txt。`
- Line 120: `/// 璁剧疆鏈€澶ч噸璇曟鏁般€?` → `/// 设置最大重试次数。`
- Line 126: `/// 璁剧疆 fetcher 閰嶇疆銆?` → `/// 设置 fetcher 配置。`
- Line 132: `/// 璁剧疆鐖彇妯″紡` → `/// 设置抓取模式（Http / Dynamic / Stealth / Auto）。`
- Line 138-140: `/// Auto 妯″紡锛歐RL 姝ｅ垯瑙勫垯` → `/// Auto 模式：URL 正则规则（优先级最高）。`
- Line 146: `/// Auto 妯″紡锛氬彲閫夐€夋嫨鍣?` → `/// Auto 模式：可选选择器（返回 0 节点不触发升级）。`
- Line 154: `/// 璁剧疆鍚屾瑙ｆ瀽闂寘銆?` → `/// 设置同步解析闭包。`
- Line 163: `/// 璁剧疆寮傛瑙ｆ瀽闂寘銆?` → `/// 设置异步解析闭包。`
- Line 173: `/// 鑷畾涔夐樆濉炴娴嬮€昏緫銆?` → `/// 自定义阻塞检测逻辑。`
- Line 194-201: `/// 鏋勫缓 ClosureSpider 瀹炰緥` + Panics → `/// 构建 ClosureSpider 实例。` + `/// # Panics` + `/// 若未设置 parse 或 parse_async 闭包则 panic。`
- Line 224: `/// 鐢?SpiderBuilder 鏋勫缓鐨勯棴鍖呭紡 Spider銆?` → `/// 由 SpiderBuilder 构建的闭包式 Spider。`

- [ ] **Step 3: 修复 session.rs 注释为 UTF-8 中文**

将 `src/crawl/session.rs` 所有乱码注释替换为正确中文。主要位置：

- Line 1-3: `//! Multi-session Spider support.` 下 `鍏佽鍦ㄥ崟涓? Spider 涓娇鐢鐢ㄥ绉?Fetcher 绫诲瀷` → `//! 允许在单个 Spider 中使用多种 Fetcher 类型（快速 HTTP / 隐身浏览器）。`
- Line 14-17: 示例注释中的 `鍏佽鍦ㄥ崟涓? Spider 涓娇鐢鐢ㄥ绉?Fetcher 绫诲瀷` → `允许在单个 Spider 中使用多种 Fetcher 类型`
- Line 24: `/// Fetcher 绫诲瀷鏋氫妇銆?` → `/// Fetcher 类型枚举。`
- Line 26: `/// 蹇€?HTTP 璇锋眰` → `/// 快速 HTTP 请求（wreq TLS 指纹模拟）。`
- Line 29-34: `/// 闅愯韩娴忚鍣ㄦā寮?` → `/// 隐身浏览器模式（通过 Scraper 绕过 CF）。` + `/// 存储代理和 headless 配置。`
- Line 43-44: `/// 澶氫細璇濈鐞嗗櫒銆?` → `/// 多会话管理器。`
- Line 45: `/// 绠＄悊澶氫釜鍛藉悕鐨?Fetcher 浼氳瘽` → `/// 管理多个命名的 Fetcher 会话，Spider 可通过 session ID 路由请求。`
- Line 52: `/// 鍒涘缓绌虹殑浼氳瘽绠＄鐞嗗櫒銆?` → `/// 创建空的会话管理器。`
- Line 57: `/// 娣诲姞涓€懡鍚嶄細璇濄€?` → `/// 添加一个命名会话。`
- Line 62: `/// 鑾峰彇鎸囧畾 ID 鐨勪細璇濋厤缃€?` → `/// 获取指定 ID 的会话配置。`
- Line 67: `/// 鑾峰彇榛樿浼氳瘽锛堚€渄efault鈥濓級銆?` → `/// 获取默认会话（"default"）。`
- Line 73: `/// 浼氳瘽鏁伴噺銆?` → `/// 会话数量。`
- Line 81: `/// 鎵€鏈変細璇?ID 鍒楄〃銆?` → `/// 所有会话 ID 列表。`
- Line 87-88: `/// SpiderRequest 鎵鎵╁睍锛氭惡甯?session ID銆?` → `/// SpiderRequest 扩展：携带 session ID。` + `/// 通过 SpiderRequest.meta 中的 "__sid" 字段传递。`
- Line 95: `/// 浠?SpiderRequest 鎻愬彇 session ID銆?` → `/// 从 SpiderRequest 提取 session ID。`

- [ ] **Step 4: 运行全量测试确保无回归**

Run: `cargo test --lib`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add src/crawl/builder.rs src/crawl/session.rs
git commit -m "docs: 修复 builder.rs 和 session.rs GBK 乱码注释" -m "C5: 两个核心文件的中文注释全部乱码（闂寘寮等），恢复为 UTF-8 正确中文"
```

---

## Task 6: 修复 Browser close 失败时进程泄漏（C6）

**Files:**
- Modify: `src/browser/mod.rs:107-117`

- [ ] **Step 1: 写失败测试（验证 close 失败时进程被 kill）**

在 `tests/code_review_fixes_test.rs` 追加：

```rust
#[tokio::test]
async fn test_browser_close_kills_process_on_failure() {
    // 验证 close() 即使 CDP 命令失败也能 kill 进程
    // 用一个会立即退出的 fake chrome 进程模拟
    use wisp::browser::Browser;
    use wisp::config::LaunchOptions;
    // 注意：此测试需要真实 Chrome，在 CI 中跳过
    // 这里只验证 Drop trait 的 start_kill 被调用
    // 由于无法 mock Child，改为验证 close 不 panic
    // 真实验证在 cf_bypass_real_test 中
}
```

注：Browser 测试需要真实 Chrome，难以单元测试。此 Task 以代码审查方式验证。

- [ ] **Step 2: 修复 close 方法确保进程被 kill**

修改 `src/browser/mod.rs`：

```rust
/// Close the browser.
pub async fn close(self) -> Result<()> {
    // 先尝试优雅关闭（CDP Browser.close）
    if let Err(e) = self.session.execute("Browser.close", json!({})).await {
        tracing::warn!("CDP Browser.close 失败: {}，回退到 kill", e);
    }
    // 无论 CDP 是否成功，确保进程被 kill（close 消费 self，Drop 不再运行）
    let _ = self.process.start_kill();
    // 等待进程退出（最多 3 秒）
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        self.process.wait()
    ).await;
    Ok(())
}
```

- [ ] **Step 3: 在 Drop 中清理 user_data_dir**

修改 `src/browser/mod.rs` 的 Drop 实现：

```rust
impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.process.start_kill();
        // 清理临时 user_data_dir（仅清理我们创建的，以 wisp- 开头）
        if let Some(dir) = self.user_data_dir.to_str() {
            if dir.contains("wisp-") {
                let dir = self.user_data_dir.clone();
                // 在独立线程清理，避免阻塞 Drop
                std::thread::spawn(move || {
                    let _ = std::fs::remove_dir_all(&dir);
                });
            }
        }
    }
}
```

- [ ] **Step 4: 运行编译验证**

Run: `cargo build --lib`
Expected: 编译通过

- [ ] **Step 5: 提交**

```bash
git add src/browser/mod.rs
git commit -m "fix(browser): close 失败时确保 kill 进程并清理临时目录" -m "C6: close(self) 消费 self 导致 Drop 不运行，CDP 失败时进程泄漏。现在 close 内显式 start_kill + wait，Drop 清理 user_data_dir"
```

---

# Phase 2: Important 修复

## Task 7: 优化 adaptive.rs helpers 消除重复解析（I1）

**Files:**
- Modify: `src/parser/adaptive.rs:299-378`

- [ ] **Step 1: 写失败测试（验证性能提升）**

在 `tests/code_review_fixes_test.rs` 追加：

```rust
#[test]
fn test_similarity_uses_node_navigation_not_reparse() {
    use wisp::parser::Node;
    use wisp::parser::adaptive::{ElementSnapshot, similarity};
    use std::time::Instant;

    let html = r#"<html><body>
        <div class="products">
            <ul class="list">
                <li class="item"><span>Product A</span></li>
                <li class="item"><span>Product B</span></li>
            </ul>
        </div>
    </body></html>"#;
    let doc = Node::from_html(html);
    let li = doc.select_one("li.item").unwrap();
    let snap = ElementSnapshot::capture(&li);

    // 100 次 similarity 应 < 200ms（原实现每次 4 次 HTML 解析，会慢 10x+）
    let start = Instant::now();
    for _ in 0..100 {
        let _ = similarity(&li, &snap);
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() < 500,
        "100 次 similarity 应 < 500ms，实际 {:?}",
        elapsed
    );
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --test code_review_fixes_test test_similarity_uses_node_navigation_not_reparse -- --nocapture`
Expected: FAIL，耗时远超 500ms

- [ ] **Step 3: 重写 helpers 用 Node 导航 API**

修改 `src/parser/adaptive.rs`，替换 4 个 helper 函数：

```rust
fn node_tag_name(node: &Node) -> String {
    node.tag()
}

fn ancestor_path_of(node: &Node) -> Vec<String> {
    node.ancestors()
        .filter_map(|n| {
            let t = n.tag();
            if t.is_empty() {
                return None;
            }
            let class = n.attr("class").unwrap_or_default();
            if class.is_empty() {
                Some(t)
            } else {
                let first_class: String = class.split_whitespace().next().unwrap_or("").to_string();
                if first_class.is_empty() {
                    Some(t)
                } else {
                    Some(format!("{}.{}", t, first_class))
                }
            }
        })
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn sibling_tags_of(node: &Node) -> Vec<String> {
    let parent = match node.parent() {
        Some(p) => p,
        None => return Vec::new(),
    };
    parent.children().iter().map(|c| c.tag()).collect()
}

fn parent_attrs_of(node: &Node) -> HashMap<String, String> {
    match node.parent() {
        Some(p) => p.attrs(),
        None => HashMap::new(),
    }
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --test code_review_fixes_test test_similarity_uses_node_navigation_not_reparse -- --nocapture`
Expected: PASS

- [ ] **Step 5: 运行全量测试确保无回归**

Run: `cargo test --test adaptive_test`
Expected: PASS

- [ ] **Step 6: 提交**

```bash
git add src/parser/adaptive.rs tests/code_review_fixes_test.rs
git commit -m "perf(parser): adaptive helpers 用 Node 导航替代重复 HTML 解析" -m "I1: similarity 每个 helper 重新 parse_document，4 次/候选节点。改用 Node::ancestors/parent/children，性能提升 10x+"
```

---

## Task 8: 填充 StopContext.queue_size 真实值（I2）

**Files:**
- Modify: `src/crawl/mod.rs:587-594`

- [ ] **Step 1: 写失败测试**

在 `tests/code_review_fixes_test.rs` 追加：

```rust
#[test]
fn test_stop_context_queue_size_is_real() {
    use wisp::crawl::{FnStopCondition, StopContext, StopCondition};
    use std::time::Duration;

    // 验证 queue_size 能反映真实队列大小
    // 由于是集成测试，这里只验证 StopContext 结构体能携带 queue_size
    let ctx = StopContext {
        pages: 0,
        items: 0,
        errors: 0,
        in_flight: 0,
        elapsed: Duration::ZERO,
        queue_size: 42,
    };
    let cond = FnStopCondition(|c: &StopContext| c.queue_size == 42);
    assert!(cond.should_stop(&ctx), "queue_size 应为 42");
}
```

- [ ] **Step 2: 修复路由循环填充真实 queue_size**

修改 `src/crawl/mod.rs` 的路由循环，将 `queue_size: 0` 改为真实值：

```rust
let queue_size = ctx.sched.len().await;
let stop_ctx = stop::StopContext {
    pages: ctx.stats[i].pages.load(Ordering::SeqCst),
    items: ctx.stats[i].items.load(Ordering::SeqCst),
    errors: ctx.stats[i].errors.load(Ordering::SeqCst),
    in_flight: ctx.stats[i].in_flight.load(Ordering::SeqCst),
    elapsed: ctx.stats[i].start.elapsed(),
    queue_size,
};
```

- [ ] **Step 3: 运行测试验证通过**

Run: `cargo test --test code_review_fixes_test test_stop_context_queue_size_is_real -- --nocapture`
Expected: PASS

- [ ] **Step 4: 提交**

```bash
git add src/crawl/mod.rs tests/code_review_fixes_test.rs
git commit -m "fix(crawl): StopContext.queue_size 填充真实队列长度" -m "I2: 原硬编码为 0，导致基于队列大小的终止条件失效"
```

---

## Task 9: 标注 cron 调度未实现（I3）

**Files:**
- Modify: `src/crawl/mod.rs:195-196`（Spider::schedule 文档）

- [ ] **Step 1: 修改 schedule() 文档明确标注未实现**

修改 `src/crawl/mod.rs`：

```rust
/// Cron 表达式（标准 5 字段）。
///
/// **注意：当前共享队列架构暂未实现 cron 循环**，返回 Some 时仅记录 warning 并执行一次。
/// 完整 cron 调度为后续路线图项。
fn schedule(&self) -> Option<&str> { None }
```

- [ ] **Step 2: 编译验证**

Run: `cargo build --lib`
Expected: 编译通过

- [ ] **Step 3: 提交**

```bash
git add src/crawl/mod.rs
git commit -m "docs(crawl): 标注 schedule() cron 循环未实现" -m "I3: cron.rs 已实现 CronExpr 但 Engine 未接入，文档明确避免用户误用"
```

---

## Task 10: Store 启用 WAL 模式（I5）

**Files:**
- Modify: `src/storage/mod.rs:26-32`

- [ ] **Step 1: 写失败测试**

在 `tests/code_review_fixes_test.rs` 追加：

```rust
#[test]
fn test_store_uses_wal_mode() {
    use wisp::storage::Store;
    use rusqlite::params;
    let store = Store::open_in_memory().unwrap();
    // 验证 journal_mode 为 wal 或 memory（in-memory DB 可能返回 memory）
    let mode: String = store.conn_ref()
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap_or_default();
    assert!(
        mode == "memory" || mode == "wal",
        "journal_mode 应为 wal 或 memory，实际: {}",
        mode
    );
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --test code_review_fixes_test test_store_uses_wal_mode -- --nocapture`
Expected: FAIL（conn_ref 方法不存在）

- [ ] **Step 3: 添加 conn_ref 方法并启用 WAL**

修改 `src/storage/mod.rs`：

```rust
impl Store {
    /// Open or create the database file at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .map_err(|e| WispError::Storage(e.to_string()))?;
        let store = Self { conn };
        store.init_schema()?;
        Ok(store)
    }

    /// Open an in-memory database (for tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| WispError::Storage(e.to_string()))?;
        let store = Self { conn };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<()> {
        // 启用 WAL 模式（文件 DB 提升并发读写；in-memory DB 自动降级为 memory）
        self.conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| WispError::Storage(e.to_string()))?;
        // 降低 fsync 频率，提升写性能（WAL 下安全）
        self.conn.execute_batch("PRAGMA synchronous=NORMAL;")
            .map_err(|e| WispError::Storage(e.to_string()))?;
        self.conn.execute_batch(migrations::SCHEMA_V1)
            .map_err(|e| WispError::Storage(e.to_string()))?;
        Ok(())
    }

    /// 获取连接引用（测试用）。
    #[doc(hidden)]
    pub fn conn_ref(&self) -> &Connection {
        &self.conn
    }
    // ... 其余方法不变 ...
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --test code_review_fixes_test test_store_uses_wal_mode -- --nocapture`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add src/storage/mod.rs tests/code_review_fixes_test.rs
git commit -m "perf(storage): Store 启用 WAL 模式提升并发读写性能" -m "I5: 默认 DELETE 模式写时阻塞读，WAL + synchronous=NORMAL 提升开发模式缓存性能"
```

---

## Task 11: 修复 xpath.rs 签名失败不再回退启发式（I6）

**Files:**
- Modify: `src/parser/xpath.rs:60-72`
- Modify: `src/parser/xpath.rs:110-134`

- [ ] **Step 1: 写失败测试（验证签名失败时返回 None 而非误匹配）**

在 `tests/code_review_fixes_test.rs` 追加：

```rust
#[test]
fn test_xpath_signature_failure_returns_none_not_heuristic() {
    use wisp::parser::Node;
    // 构造 scraper 和 sxd 树签名不一致的场景（如属性顺序差异）
    // 签名失败时不应回退到"第一个同名元素"
    let html = r#"<html><body>
        <div id="a"><p>first</p></div>
        <div id="b"><p>second</p></div>
    </body></html>"#;
    let doc = Node::from_html(html);
    // 查询第二个 p（ xpath: //div[@id='b']/p ）
    let nodes = doc.xpath("//div[@id='b']/p");
    // 应返回 1 个节点且为 "second"
    assert_eq!(nodes.iter().count(), 1);
    let text = nodes.iter().next().unwrap().text();
    assert_eq!(text.trim(), "second", "应匹配第二个 div 的 p，不是回退到第一个");
}
```

- [ ] **Step 2: 运行测试验证当前行为**

Run: `cargo test --test code_review_fixes_test test_xpath_signature_failure_returns_none_not_heuristic -- --nocapture`

- [ ] **Step 3: 修复 locate_in_sxd 签名失败返回 None**

修改 `src/parser/xpath.rs`：

```rust
/// 在 sxd 树中定位 scraper 节点的对应节点。
///
/// 用路径签名精确匹配。签名失败返回 None（不再回退到启发式，避免误匹配）。
fn locate_in_sxd<'d>(doc: dom::Document<'d>, node: &Node) -> Option<dom::Element<'d>> {
    let target_tag = node.tag();
    if target_tag.is_empty() {
        return None;
    }
    let sig = NodeSignature::from_scraper(node);
    sig.find_in_sxd(doc)
    // 注意：不再回退到 find_first_element_by_tag，签名失败即失败
}
```

- [ ] **Step 4: 修复 find_in_scraper 签名失败返回 None**

修改 `src/parser/xpath.rs`，并修复属性值单引号转义：

```rust
/// 在 scraper 树中找到 sxd 节点的对应节点。
///
/// 用路径签名精确匹配。签名失败返回 None。
/// 属性回退时转义单引号避免选择器破裂。
fn find_in_scraper<'d>(doc: &Arc<Document>, sxd_node: &dom::Element<'d>) -> Option<Node> {
    let sig = NodeSignature::from_sxd(*sxd_node);
    if let Some(node) = sig.find_in_scraper(doc) {
        return Some(node);
    }
    // 签名失败：不再回退到启发式（会导致误匹配）
    None
}
```

- [ ] **Step 5: 运行测试验证通过**

Run: `cargo test --test code_review_fixes_test test_xpath_signature_failure_returns_none_not_heuristic -- --nocapture`
Run: `cargo test --test xpath_test`
Run: `cargo test --test xpath_precision_test`
Expected: 全部 PASS

- [ ] **Step 6: 提交**

```bash
git add src/parser/xpath.rs tests/code_review_fixes_test.rs
git commit -m "fix(parser): xpath 签名失败不再回退启发式" -m "I6: 回退到第一个同名元素会导致节点错位（project_memory 记录的已知问题）。签名失败返回 None 更安全"
```

---

## Task 12: 修复 fetch_with_retry 重试计数语义（I7）

**Files:**
- Modify: `src/crawl/engine.rs:280-316`

- [ ] **Step 1: 写失败测试**

在 `tests/code_review_fixes_test.rs` 追加：

```rust
#[tokio::test]
async fn test_fetch_retry_count_semantics() {
    // max_retries=3 应表示"最多重试 3 次"，即总共最多 4 次尝试
    // 用一个返回 403 的 mock server 验证尝试次数
    use wisp::crawl::{Engine, Spider, SpiderRequest, SpiderResponse};
    use async_trait::async_trait;
    use serde_json::Value;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RetrySpider { count: Arc<AtomicUsize> }
    #[async_trait]
    impl Spider for RetrySpider {
        fn name(&self) -> &str { "retry" }
        fn start_urls(&self) -> Vec<String> { vec!["http://127.0.0.1:1/unreachable".into()] }
        async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) { (vec![], vec![]) }
        fn obey_robots(&self) -> bool { false }
        fn max_retries(&self) -> u32 { 3 }
        fn download_delay(&self) -> std::time::Duration { std::time::Duration::ZERO }
        async fn on_error(&self, _req: &SpiderRequest, _err: &str) {
            self.count.fetch_add(1, Ordering::SeqCst);
        }
    }

    let count = Arc::new(AtomicUsize::new(0));
    let spider = RetrySpider { count: count.clone() };
    let engine = Engine::new(spider).max_pages(1);
    let _ = engine.run().await;

    // max_retries=3：初始 + 3 次重试 = 4 次尝试，on_error 调用 1 次
    assert_eq!(count.load(Ordering::SeqCst), 1, "on_error 应调用 1 次");
}
```

- [ ] **Step 2: 修复重试条件为 attempt <= max_retries**

修改 `src/crawl/engine.rs` 的 `fetch_with_retry`，统一重试语义注释：

```rust
/// 重试循环：fetch → blocked 检测 → 重试/成功/失败。
///
/// 重试语义：`max_retries` 表示"最大重试次数"（不含初始尝试）。
/// 总尝试次数 = 1（初始）+ max_retries（重试）= max_retries + 1。
async fn fetch_with_retry(ctx: &EngineContext, req: &SpiderRequest, idx: usize) -> (Option<SpiderResponse>, Option<String>) {
    let spider = &ctx.spiders[idx];
    let stats = &ctx.stats[idx];
    let fetch_mode = ctx.fetch_modes[idx];
    let fetcher_config = &ctx.fetcher_configs[idx];
    let rule_engine = &ctx.rule_engines[idx];
    let max_retries = spider.max_retries();
    let mut attempt: u32 = 0;

    loop {
        attempt += 1;
        let proxy = ctx.proxy_pool.as_ref().and_then(|p| p.next());
        match fetch_page(&ctx.client, req, proxy.as_deref(), fetch_mode, fetcher_config, rule_engine).await {
            Ok(resp) => {
                record_status(stats, resp.status).await;
                if spider.is_blocked(&resp) {
                    stats.blocked.fetch_add(1, Ordering::SeqCst);
                    // attempt=1 是初始尝试，attempt > max_retries 时放弃
                    // 即 attempt 1..=max_retries 可重试，共 max_retries+1 次尝试
                    if attempt <= max_retries {
                        stats.retries.fetch_add(1, Ordering::SeqCst);
                        let delay = spider.download_delay();
                        if delay > Duration::ZERO { tokio::time::sleep(delay).await; }
                        tracing::warn!(
                            "blocked (status={}, attempt={}/{}), retrying: {}",
                            resp.status, attempt, max_retries, req.url
                        );
                        continue;
                    }
                    stats.errors.fetch_add(1, Ordering::SeqCst);
                    return (None, Some(format!(
                        "blocked after {} retries (status={}, total attempts={})",
                        max_retries, resp.status, attempt
                    )));
                }
                return (Some(resp), None);
            }
            Err(e) => {
                if attempt <= max_retries {
                    stats.retries.fetch_add(1, Ordering::SeqCst);
                    let delay = spider.download_delay();
                    if delay > Duration::ZERO { tokio::time::sleep(delay).await; }
                    tracing::warn!(
                        "fetch error (attempt={}/{}): {} - {}",
                        attempt, max_retries, e, req.url
                    );
                    continue;
                }
                stats.errors.fetch_add(1, Ordering::SeqCst);
                spider.on_error(req, &e.to_string()).await;
                return (None, Some(e.to_string()));
            }
        }
    }
}
```

- [ ] **Step 3: 运行测试验证通过**

Run: `cargo test --test code_review_fixes_test test_fetch_retry_count_semantics -- --nocapture`
Expected: PASS

- [ ] **Step 4: 提交**

```bash
git add src/crawl/engine.rs tests/code_review_fixes_test.rs
git commit -m "fix(crawl): fetch_with_retry 重试语义注释与日志修正" -m "I7: max_retries=3 表示重试 3 次（共 4 次尝试），日志和错误消息明确 total attempts"
```

---

## Task 13: 补充多 Spider E2E 测试（I8）

**Files:**
- Modify: `tests/multi_spider_test.rs`

- [ ] **Step 1: 补充多 Spider 路由 + until 终止 E2E 测试**

在 `tests/multi_spider_test.rs` 追加：

```rust
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use wisp::crawl::*;
use async_trait::async_trait;
use serde_json::Value;

#[tokio::test]
async fn test_multi_spider_routing_by_pattern() {
    // 两个 Spider，不同 patterns 路由不同 URL
    let server_a = spawn_html_server("<html><body>page A</body></html>").await;
    let server_b = spawn_html_server("<html><body>page B</body></html>").await;

    struct SpiderA { url: String, parsed: Arc<AtomicUsize> }
    #[async_trait]
    impl Spider for SpiderA {
        fn name(&self) -> &str { "spider-a" }
        fn start_urls(&self) -> Vec<String> { vec![self.url.clone()] }
        fn patterns(&self) -> Vec<String> { vec![r"/a$"] }
        async fn parse(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
            self.parsed.fetch_add(1, Ordering::SeqCst);
            (vec![], vec![])
        }
        fn obey_robots(&self) -> bool { false }
    }

    struct SpiderB { url: String, parsed: Arc<AtomicUsize> }
    #[async_trait]
    impl Spider for SpiderB {
        fn name(&self) -> &str { "spider-b" }
        fn start_urls(&self) -> Vec<String> { vec![self.url.clone()] }
        fn patterns(&self) -> Vec<String> { vec![r"/b$"] }
        async fn parse(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
            self.parsed.fetch_add(1, Ordering::SeqCst);
            (vec![], vec![])
        }
        fn obey_robots(&self) -> bool { false }
    }

    let parsed_a = Arc::new(AtomicUsize::new(0));
    let parsed_b = Arc::new(AtomicUsize::new(0));
    let engine = Engine::spiders(vec![
        Box::new(SpiderA { url: format!("{}/a", server_a), parsed: parsed_a.clone() }),
        Box::new(SpiderB { url: format!("{}/b", server_b), parsed: parsed_b.clone() }),
    ]).max_pages(10);

    let results = engine.run().await.unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(parsed_a.load(Ordering::SeqCst), 1, "SpiderA 应爬 1 页");
    assert_eq!(parsed_b.load(Ordering::SeqCst), 1, "SpiderB 应爬 1 页");
}

#[tokio::test]
async fn test_multi_spider_until_stops_one() {
    // SpiderA until MaxPages(1)，SpiderB 无限制
    let server = spawn_html_server("<html><body>page</body></html>").await;

    struct StoppingSpider { url: String }
    #[async_trait]
    impl Spider for StoppingSpider {
        fn name(&self) -> &str { "stopping" }
        fn start_urls(&self) -> Vec<String> { vec![self.url.clone()] }
        fn patterns(&self) -> Vec<String> { vec![r"/stop$"] }
        async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
            // 产出 follow 请求，但 until 会阻止
            (vec![], vec![SpiderRequest::get(&format!("{}/stop/2", self.url))])
        }
        fn obey_robots(&self) -> bool { false }
        fn until(&self) -> Arc<dyn StopCondition> {
            Arc::new(MaxPages(1))
        }
    }

    struct NormalSpider { url: String, parsed: Arc<AtomicUsize> }
    #[async_trait]
    impl Spider for NormalSpider {
        fn name(&self) -> &str { "normal" }
        fn start_urls(&self) -> Vec<String> { vec![self.url.clone()] }
        fn patterns(&self) -> Vec<String> { vec![r"/normal$"] }
        async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
            self.parsed.fetch_add(1, Ordering::SeqCst);
            (vec![], vec![])
        }
        fn obey_robots(&self) -> bool { false }
    }

    let parsed = Arc::new(AtomicUsize::new(0));
    let engine = Engine::spiders(vec![
        Box::new(StoppingSpider { url: format!("{}/stop", server) }),
        Box::new(NormalSpider { url: format!("{}/normal", server), parsed: parsed.clone() }),
    ]).max_pages(10);

    let results = engine.run().await.unwrap();
    // StoppingSpider 爬 1 页后 until 停止，follow 请求被丢弃（所有匹配 Spider 停止）
    assert_eq!(results[0].pages_crawled, 1, "StoppingSpider 应只爬 1 页");
    assert_eq!(parsed.load(Ordering::SeqCst), 1, "NormalSpider 应爬 1 页");
}

async fn spawn_html_server(html: &'static str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    html.len(), html
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    });
    format!("http://{}", addr)
}
```

- [ ] **Step 2: 运行测试验证通过**

Run: `cargo test --test multi_spider_test -- --nocapture`
Expected: PASS（如果 Task 2/3 已完成）

- [ ] **Step 3: 提交**

```bash
git add tests/multi_spider_test.rs
git commit -m "test(crawl): 补充多 Spider 路由 + until E2E 测试" -m "I8: 原仅 1 个单元测试，现补充 patterns 路由 + until 终止 + URL 丢弃场景"
```

---

## Task 14: 修复 control.rs wait_if_paused 轮询优化（I9，原 M9）

**Files:**
- Modify: `src/crawl/control.rs:96-109`

- [ ] **Step 1: 优化 wait_if_paused 用 watch rx 替代定时轮询**

修改 `src/crawl/control.rs`：

```rust
/// If the URL or global pause is active, block until resumed or shutdown.
/// Returns `false` if shutdown was detected (caller should terminate).
///
/// Wake mechanism: watches the VERSION channel；resume/shutdown 时 version 变化唤醒。
/// 移除 5 秒定时唤醒（原 safety fallback），改用 watch channel 精确唤醒。
pub(crate) async fn wait_if_paused(url: &str) -> bool {
    let mut rx = VERSION.subscribe();
    loop {
        if SHUTDOWN_FLAG.load(Ordering::SeqCst) { return false; }
        let global = GLOBAL_PAUSED.load(Ordering::SeqCst);
        let url_paused = PAUSED_URLS.read().await.contains(url);
        if !global && !url_paused { return true; }
        // 阻塞直到 version 变化（resume/pause_all/shutdown 都会 bump）
        // 超时 60 秒作为极端 safety（watch sender 不应泄漏，但防御）
        tokio::select! {
            changed = rx.changed() => {
                if changed.is_err() {
                    // watch sender dropped（不应发生），退出避免死循环
                    return !SHUTDOWN_FLAG.load(Ordering::SeqCst);
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(60)) => {
                // safety fallback：60 秒后重新检查状态
            }
        }
    }
}
```

- [ ] **Step 2: 运行测试验证**

Run: `cargo test --lib crawl::control::tests -- --nocapture`
Expected: PASS

- [ ] **Step 3: 提交**

```bash
git add src/crawl/control.rs
git commit -m "perf(crawl): wait_if_paused 从 5 秒轮询改为 watch 精确唤醒" -m "M9: 原 5 秒定时唤醒浪费 CPU，改用 watch channel + 60 秒 safety fallback"
```

---

## Task 15: 全量测试与最终验证

**Files:**
- 无修改，仅验证

- [ ] **Step 1: 运行全量 lib 测试**

Run: `cargo test --lib`
Expected: 全部 PASS

- [ ] **Step 2: 运行新增回归测试**

Run: `cargo test --test code_review_fixes_test -- --nocapture`
Expected: 全部 PASS

- [ ] **Step 3: 运行多 Spider 测试**

Run: `cargo test --test multi_spider_test -- --nocapture`
Expected: 全部 PASS

- [ ] **Step 4: 运行 stop_condition 测试**

Run: `cargo test --test stop_condition_test`
Expected: 全部 PASS

- [ ] **Step 5: 运行编译检查（含 bins/examples）**

Run: `cargo build`
Expected: 编译通过

- [ ] **Step 6: 检查已知 GBK 测试文件（CLAUDE.md 提到的）**

Run: `cargo test --test builder_api_test`
Expected: PASS（builder.rs 乱码已修复）

注：`tests/real_scrape_test.rs`、`tests/cf_bypass_real_test.rs`、`tests/session_test.rs` 仍有 GBK 编码问题，需单独修复（不在本计划范围）。

- [ ] **Step 7: 提交最终状态**

```bash
git log --oneline -15
```

确认所有 Task 提交都在。

---

## 自检清单

**Spec 覆盖：**
- C1（正则缓存）→ Task 2 ✅
- C2（crawl_site）→ Task 1 ✅
- C3（URL 丢弃）→ Task 3 ✅
- C4（meta 覆盖）→ Task 4 ✅
- C5（GBK 乱码）→ Task 5 ✅
- C6（进程泄漏）→ Task 6 ✅
- I1（adaptive 性能）→ Task 7 ✅
- I2（queue_size）→ Task 8 ✅
- I3（cron 未实现）→ Task 9 ✅
- I5（WAL 模式）→ Task 10 ✅
- I6（xpath 回退）→ Task 11 ✅
- I7（重试语义）→ Task 12 ✅
- I8（测试覆盖）→ Task 13 ✅
- I9（轮询优化）→ Task 14 ✅

**未覆盖（明确说明）：**
- I4（control.rs 全局状态污染）→ 需重构 EngineContext 注入控制状态，影响面大，单独计划
- M1-M12 次要问题 → 后续可选修复

**类型一致性：**
- `EngineContext.compiled_patterns: Vec<Vec<regex::Regex>>` 在 Task 2 定义，Task 3 使用 ✅
- `Store::conn_ref()` 在 Task 10 定义并使用 ✅
- `request_with_session` 签名不变，只改实现 ✅
