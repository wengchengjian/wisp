# Wisp 爬虫增强设计规格

## 概述

将 wisp 从纯浏览器自动化工具增强为完整的爬虫框架，一比一复刻 rust_scrapling 的全部能力（parser/fetchers/spiders/core），但使用更优雅的命名和 API 设计。添加 criterion 基准测试。

## 模块映射

| rust_scrapling | wisp | 说明 |
|---|---|---|
| `parser::Selector` | `parser::Node` | 单个 DOM 节点包装 |
| `parser::Selectors` | `parser::NodeList` | 节点集合 |
| `parser::adaptive` | `parser::adaptive` | 自适应元素重定位 |
| `parser::selector_generation` | `parser::generate` | CSS/XPath 选择器自动生成 |
| `fetchers::Fetcher` | `fetch::Client` | HTTP 客户端 |
| `fetchers::FetcherConfig` | `fetch::Config` | 客户端配置 |
| `fetchers::Response` | `fetch::Response` | HTTP 响应 |
| `fetchers::encoding` | `fetch::encoding` | 字符编码检测 |
| `fetchers::proxy` | `fetch::proxy` | 代理配置 |
| `spiders::Spider` | `crawl::Spider` | 爬虫 trait |
| `spiders::CrawlerEngine` | `crawl::Engine` | 爬取引擎 |
| `spiders::scheduler` | `crawl::scheduler` | URL 调度 + 去重 |
| `spiders::robots` | `crawl::robots` | robots.txt 解析 |
| `spiders::cache` | `crawl::cache` | 请求缓存 |
| `spiders::templates` | `crawl::templates` | 模板爬虫 |
| `core::TextHandler` | `text::Text` | 文本处理 |
| `core::AttributesHandler` | `text::Attrs` | 属性处理 |

## 目录结构

```
src/
├── parser/
│   ├── mod.rs          Node + NodeList 公共 API
│   ├── adaptive.rs     自适应元素重定位（基于相似度匹配）
│   └── generate.rs     CSS/XPath 选择器自动生成
├── fetch/
│   ├── mod.rs          Client + Config + Response
│   ├── encoding.rs     字符编码检测（encoding_rs）
│   └── proxy.rs        代理配置解析
├── crawl/
│   ├── mod.rs          Spider trait + Engine + SpiderRequest/Response
│   ├── scheduler.rs    URL 调度器（优先级队列 + 指纹去重）
│   ├── robots.rs       robots.txt 解析与遵守
│   ├── cache.rs        请求/响应缓存（SQLite 或文件）
│   └── templates.rs    CrawlSpider + SitemapSpider 模板
├── text/
│   └── mod.rs          Text（正则/清理/提取）+ Attrs（属性操作）
├── ...existing modules (browser, page, cdp, challenge, human, proxy, scraper)...
benches/
└── bench.rs            criterion 基准测试
```

## 模块 1: parser（HTML 解析）

### Node（对应 rust_scrapling::Selector）

```rust
pub struct Node { /* html5ever tree + node id */ }

impl Node {
    // 构造
    pub fn from_html(html: &str) -> Self;
    pub fn from_file(path: &str) -> Result<Self>;

    // CSS 选择器
    pub fn select(&self, css: &str) -> NodeList;
    pub fn select_one(&self, css: &str) -> Option<Node>;

    // XPath
    pub fn xpath(&self, expr: &str) -> NodeList;

    // 内容提取
    pub fn text(&self) -> String;
    pub fn html(&self) -> String;
    pub fn outer_html(&self) -> String;
    pub fn attr(&self, name: &str) -> Option<String>;
    pub fn attrs(&self) -> HashMap<String, String>;

    // DOM 导航
    pub fn parent(&self) -> Option<Node>;
    pub fn children(&self) -> NodeList;
    pub fn next_sibling(&self) -> Option<Node>;
    pub fn prev_sibling(&self) -> Option<Node>;
    pub fn first_child(&self) -> Option<Node>;
    pub fn last_child(&self) -> Option<Node>;

    // 高级
    pub fn contains_text(&self, text: &str) -> bool;
    pub fn matches(&self, css: &str) -> bool;
    pub fn generate_selector(&self) -> String;   // 自动生成唯一 CSS
    pub fn generate_xpath(&self) -> String;      // 自动生成唯一 XPath

    // 文本处理（集成 text 模块）
    pub fn text_clean(&self) -> String;          // 清理空白
    pub fn text_regex(&self, pattern: &str) -> Vec<String>;
}
```

### NodeList（对应 rust_scrapling::Selectors）

```rust
pub struct NodeList { nodes: Vec<Node> }

impl NodeList {
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn first(&self) -> Option<&Node>;
    pub fn last(&self) -> Option<&Node>;
    pub fn get(&self, index: usize) -> Option<&Node>;
    pub fn text(&self) -> Vec<String>;
    pub fn html(&self) -> Vec<String>;
    pub fn attr(&self, name: &str) -> Vec<Option<String>>;
    pub fn select(&self, css: &str) -> NodeList;  // 链式
    pub fn filter(&self, predicate: impl Fn(&Node) -> bool) -> NodeList;
    pub fn iter(&self) -> impl Iterator<Item = &Node>;
}
```

### adaptive（自适应重定位）

当网站结构变化时，基于元素特征（标签、属性、文本、位置）的相似度匹配重新定位元素。

```rust
pub struct ElementData {
    pub tag: String,
    pub attrs: HashMap<String, String>,
    pub text_preview: String,
    pub path: Vec<String>,  // 祖先路径
}

pub fn relocate(html: &str, saved: &ElementData, tolerance: f64) -> Option<Node>;
```

## 模块 2: fetch（HTTP 客户端）

### Client（对应 rust_scrapling::Fetcher）

```rust
pub struct Client { /* reqwest::Client + config */ }

impl Client {
    pub fn builder() -> ClientBuilder;
    pub async fn get(&self, url: &str) -> Result<Response>;
    pub async fn post(&self, url: &str, body: Option<&str>, json: Option<&Value>) -> Result<Response>;
    pub async fn put(&self, url: &str, body: Option<&str>, json: Option<&Value>) -> Result<Response>;
    pub async fn delete(&self, url: &str) -> Result<Response>;
}

pub struct ClientBuilder { ... }
impl ClientBuilder {
    pub fn timeout(self, d: Duration) -> Self;
    pub fn proxy(self, url: &str) -> Self;
    pub fn user_agent(self, ua: &str) -> Self;
    pub fn headers(self, map: HashMap<String, String>) -> Self;
    pub fn follow_redirects(self, max: usize) -> Self;
    pub fn build(self) -> Result<Client>;
}
```

### Response

```rust
pub struct Response {
    pub status: u16,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn text(&self) -> Result<String>;       // 自动编码检测
    pub fn json(&self) -> Result<Value>;
    pub fn parse(&self) -> Result<Node>;        // 直接解析为 DOM
    pub fn is_ok(&self) -> bool;
}
```

## 模块 3: crawl（爬虫引擎）

### Spider trait

```rust
#[async_trait]
pub trait Spider: Send + Sync + 'static {
    // 必须实现
    fn name(&self) -> &str;
    fn start_urls(&self) -> Vec<String>;
    async fn parse(&self, response: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>);

    // 可选覆盖
    fn allowed_domains(&self) -> HashSet<String> { HashSet::new() }
    fn concurrent_requests(&self) -> u32 { 8 }
    fn download_delay(&self) -> Duration { Duration::from_millis(0) }
    fn obey_robots(&self) -> bool { true }
    fn max_retries(&self) -> u32 { 3 }
    fn fetcher_config(&self) -> fetch::Config { Default::default() }
    async fn on_start(&self) {}
    async fn on_close(&self) {}
    async fn on_error(&self, req: &SpiderRequest, err: &str) {}
    async fn on_item(&self, item: Value) -> Option<Value> { Some(item) }
    fn is_blocked(&self, resp: &SpiderResponse) -> bool { false }
}
```

### Engine

```rust
pub struct Engine<S: Spider> { spider: S, config: EngineConfig }

impl<S: Spider> Engine<S> {
    pub fn new(spider: S) -> Self;
    pub fn with_config(spider: S, config: EngineConfig) -> Self;
    pub async fn run(self) -> Result<CrawlStats>;
}

pub struct CrawlStats {
    pub items_scraped: usize,
    pub pages_crawled: usize,
    pub errors: usize,
    pub duration: Duration,
}
```

### SpiderRequest / SpiderResponse

```rust
pub struct SpiderRequest {
    pub url: String,
    pub method: Method,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
    pub meta: Value,          // 用户自定义元数据
    pub callback: Option<String>,
    pub priority: i32,
}

pub struct SpiderResponse {
    pub url: String,
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub request: SpiderRequest,
}

impl SpiderResponse {
    pub fn text(&self) -> Result<String>;
    pub fn parse(&self) -> Result<Node>;
    pub fn json(&self) -> Result<Value>;
}
```

### templates

```rust
// CrawlSpider: 基于规则的自动爬取
pub struct CrawlSpider { rules: Vec<CrawlRule>, ... }
pub struct CrawlRule { pattern: Regex, callback: String, follow: bool }

// SitemapSpider: 基于 sitemap.xml 的爬取
pub struct SitemapSpider { sitemap_urls: Vec<String>, ... }
```

## 模块 4: text（文本处理）

```rust
pub struct Text<'a>(&'a str);

impl<'a> Text<'a> {
    pub fn clean(&self) -> String;                    // 去除多余空白
    pub fn extract_regex(&self, pattern: &str) -> Vec<String>;
    pub fn extract_emails(&self) -> Vec<String>;
    pub fn extract_urls(&self) -> Vec<String>;
    pub fn truncate(&self, max: usize) -> String;
    pub fn strip_tags(&self) -> String;
}

pub struct Attrs(HashMap<String, String>);
impl Attrs {
    pub fn get(&self, name: &str) -> Option<&str>;
    pub fn to_json(&self) -> Value;
}
```

## 与现有模块集成

- `scraper::ScrapeResponse` 添加 `.parse() -> Node` 方法
- `fetch::Client` 复用现有 `proxy::ProxyPool`
- `crawl::Engine` 可选使用浏览器获取（对 CF 保护站点）

## 基准测试（benches/bench.rs）

使用 criterion 框架：

```rust
// 测试项：
1. HTML 解析速度（10KB / 100KB / 1MB 文档）
2. CSS 选择器查询（简单/复杂/嵌套）
3. XPath 查询
4. 文本提取
5. NodeList 遍历
6. 与 scraper crate 对比
```

## 新增依赖

```toml
[dependencies]
scraper = "0.23"          # html5ever + css selector 引擎
select = "0.6"            # XPath 支持（或 xpath crate）
encoding_rs = "0.8"       # 字符编码检测
url = "2"                 # URL 解析
async-trait = "0.1"       # Spider trait 异步方法

[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }

[[bench]]
name = "bench"
harness = false
```

## 实现顺序

1. `text/` - 文本处理（无依赖，最简单）
2. `parser/` - HTML 解析 + 选择器（依赖 scraper crate）
3. `fetch/` - HTTP 客户端（依赖 reqwest + encoding_rs）
4. `crawl/` - 爬虫引擎（依赖 fetch + parser）
5. `benches/` - 基准测试
6. 集成：ScrapeResponse.parse() + lib.rs re-exports
