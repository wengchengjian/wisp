# 借鉴 Scrapling 增强设计规格

## 概述

基于 wisp 当前代码与 rust_scrapling / Python Scrapling 的对比分析，补齐差距。覆盖 4 个阶段（P0 硬伤 → P1 解析/fetch 增强 → P2 工程化与 MCP → P3 长尾能力），按优先级分阶段交付，每阶段独立可测可发布。

本 spec 不推翻 [2026-07-20-wisp-scraper-enhancement-design.md](./2026-07-20-wisp-scraper-enhancement-design.md)（该 spec 的 parser/fetch/crawl/text 骨架已落地），而是针对对比分析中发现的新差距做增量设计。

## 关键决策汇总

| 维度 | 决策 |
|---|---|
| 实施方案 | 方案 A：按优先级分 4 阶段 |
| HTTP 客户端 | wreq 替换 reqwest（TLS/JA3/JA4 指纹模拟，BoringSSL） |
| adaptive 深度 | 完整移植 Python Scrapling（difflib + 上下文指纹 + SQLite） |
| MCP server | 内置于 wisp CLI，stdio 协议 |
| Spider 并发 | `buffer_unordered` 流式 |
| checkpoint | SQLite 表存储（复用统一 Store，blob 用 bincode） |
| XPath | sxd-xpath，按需懒解析 |
| Session | SQLite 持久化（HTTP + Stealthy 两种） |
| DoH | wreq `dns_resolver` API（已验证可用） |

## 模块依赖图

```
                    ┌─────────────────────────────────────────┐
                    │              bin/wisp.rs                 │
                    │  (CLI: scrape / crawl / mcp serve)      │
                    └──────────────┬──────────────────────────┘
                                   │
        ┌──────────────────────────┼──────────────────────────┐
        │                          │                          │
        ▼                          ▼                          ▼
┌───────────────┐         ┌────────────────┐        ┌────────────────┐
│  scraper/     │         │   crawl/       │        │   mcp/ (新)    │
│  高级抓取     │         │   爬虫引擎     │        │   MCP server   │
│  (浏览器)     │         │                │        │   (stdio)      │
└───────┬───────┘         └───────┬────────┘        └───────┬────────┘
        │                         │                         │
        │  ┌──────────────────────┤                         │
        │  │                      │                         │
        ▼  ▼                      ▼                         │
┌───────────────┐         ┌────────────────┐                │
│  browser/     │         │   fetch/       │                │
│  page/        │         │   HTTP 客户端  │◀───────────────┘
│  challenge/   │         │   (wreq)       │
│  human/       │         └───────┬────────┘
│  proxy/       │                 │
│  patches/     │                 │
└───────────────┘                 │
                                  │
        ┌─────────────────────────┼─────────────────────────┐
        │                         │                         │
        ▼                         ▼                         ▼
┌───────────────┐         ┌────────────────┐        ┌────────────────┐
│  parser/      │         │   text/        │        │  storage/ (新) │
│  HTML 解析    │         │   文本处理     │        │  SQLite 统一   │
│  CSS+XPath    │         │                │        │  (adaptive+    │
│  adaptive     │         └────────────────┘        │   checkpoint+  │
└───────┬───────┘                                   │   session)     │
        │                                           └────────────────┘
        │  sxd-xpath 懒解析                                 ▲
        ▼                                                   │
┌───────────────┐                                           │
│  scraper crate│                                           │
│  html5ever    │───────────────────────────────────────────┘
└───────────────┘
```

## 新增模块

- `src/storage/mod.rs` — 统一 SQLite 存储层（adaptive + checkpoint + session + cache 共用一个 db）
- `src/mcp/mod.rs` — MCP server（stdio JSON-RPC）
- `src/browser/adblock.rs` — 广告域名拦截
- `src/fetch/doh.rs` — DoH 解析器
- `src/fetch/session.rs` — HTTP Session 持久化
- `src/scraper/session.rs` — 浏览器 Session 持久化
- `data/ad_domains.json` — 内置广告域名列表（~3500 条）

## 新增依赖

```toml
[dependencies]
# 替换 reqwest → wreq（TLS 指纹模拟）
wreq = "5"
wreq-util = "2"          # 浏览器设备模拟（Chrome/Firefox）

# XPath 真实现
sxd-document = "0.3"
sxd-xpath = "0.4"

# SQLite 统一存储
rusqlite = { version = "0.32", features = ["bundled"] }

# checkpoint blob 序列化（CrawlState 用）
bincode = "1"

# 流式输出
tokio-stream = "0.1"

# 时间戳
chrono = { version = "0.4", features = ["serde"] }

# URL 解析（广告拦截用）
url = "2"
```

## 移除依赖

- `reqwest` → 被 `wreq` 替换

## 统一存储层设计

为避免 adaptive/checkpoint/session/cache 各自开 SQLite 文件，新增统一存储层：

```rust
// src/storage/mod.rs
pub struct Store {
    conn: rusqlite::Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self>;
    pub fn open_in_memory() -> Result<Self>;

    // adaptive 模块用
    pub fn save_element(&self, url: &str, key: &str, data: &ElementSnapshot) -> Result<()>;
    pub fn load_element(&self, url: &str, key: &str) -> Result<Option<ElementSnapshot>>;

    // checkpoint 模块用
    pub fn save_checkpoint(&self, state: &CrawlState) -> Result<()>;
    pub fn load_checkpoint(&self, spider_name: &str) -> Result<Option<CrawlState>>;
    pub fn delete_checkpoint(&self, spider_name: &str) -> Result<()>;

    // session 模块用（阶段 4）
    pub fn save_cookies(&self, session_id: &str, cookies: &[Cookie]) -> Result<()>;
    pub fn load_cookies(&self, session_id: &str) -> Result<Vec<Cookie>>;
    pub fn list_sessions(&self) -> Result<Vec<String>>;
    pub fn delete_session(&self, session_id: &str) -> Result<()>;

    // cache 模块用（阶段 4）
    pub fn save_cached_response(&self, url: &str, method: &str, resp: &CachedResponse) -> Result<()>;
    pub fn load_cached_response(&self, url: &str, method: &str) -> Result<Option<CachedResponse>>;
}
```

### SQLite Schema（4 张表）

```sql
-- adaptive 元素快照
CREATE TABLE IF NOT EXISTS element_snapshots (
    url TEXT NOT NULL,
    key TEXT NOT NULL,
    tag TEXT,
    attrs TEXT,              -- JSON
    text_preview TEXT,
    ancestor_path TEXT,      -- JSON array
    sibling_tags TEXT,       -- JSON array
    position_in_parent INTEGER,
    parent_tag TEXT,
    parent_attrs TEXT,       -- JSON
    captured_at INTEGER,
    PRIMARY KEY (url, key)
);

-- Spider checkpoint
CREATE TABLE IF NOT EXISTS crawl_checkpoints (
    spider_name TEXT PRIMARY KEY,
    state BLOB NOT NULL,     -- bincode 序列化的 CrawlState
    saved_at INTEGER NOT NULL
);

-- Session cookies（阶段 4）
CREATE TABLE IF NOT EXISTS session_cookies (
    session_id TEXT NOT NULL,
    name TEXT NOT NULL,
    value TEXT,
    domain TEXT,
    path TEXT,
    expires INTEGER,
    secure INTEGER,
    http_only INTEGER,
    PRIMARY KEY (session_id, name)
);

-- 响应缓存（阶段 4，replay 模式）
CREATE TABLE IF NOT EXISTS response_cache (
    url TEXT NOT NULL,
    method TEXT NOT NULL,
    status INTEGER,
    headers TEXT,            -- JSON
    body BLOB,
    cached_at INTEGER,
    PRIMARY KEY (url, method)
);
```

## 跨阶段不变量

- `Node` 的公共 API 保持向后兼容（`select/text/attr/...` 签名不变）。阶段 2 内部从 `scraper::Html` 重构为 `Arc<Document>` 共享所有权，但 API 签名不变。
- `Spider` trait 在阶段 1 加 `stream()` 默认方法，阶段 3 实现；trait 签名兼容。
- `fetch::Client` 在阶段 2 从 reqwest 切到 wreq，公共 API（get/post/builder）签名不变。
- 所有 SQLite 操作走 `storage::Store`，模块间不直接持有 `rusqlite::Connection`。

---

# 阶段 1：P0 硬伤

## 1.1 adaptive 完整移植

### 1.1.1 移植范围

| Python Scrapling 概念 | wisp 对应 | 说明 |
|---|---|---|
| `ElementSnapshot` | `ElementSnapshot` | tag + attrs + text + 祖先路径 + 兄弟标签序列 + 父节点信息 |
| `SequenceMatcher` (difflib) | `SequenceMatcher` | 自行实现，~200 行，行为对齐 Python `difflib.SequenceMatcher` |
| 相似度评分函数 | `similarity()` | 6 维加权：tag/attrs/text/path/sibling/parent |
| `auto_save` + `adaptive` 参数 | `css_adaptive(selector, key, store, auto_save, tolerance)` | API 签名与 rust_scrapling 一致 |
| SQLite 持久化 | `storage::Store::save_element/load_element` | 复用统一存储层 |

### 1.1.2 ElementSnapshot 数据结构

```rust
// src/parser/adaptive.rs
use std::collections::HashMap;
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementSnapshot {
    pub tag: String,
    pub attrs: HashMap<String, String>,
    pub text_preview: String,        // 前 200 字符
    pub ancestor_path: Vec<String>,  // ["html", "body", "div.main", "ul.products", "li"]
    pub sibling_tags: Vec<String>,   // 兄弟节点标签序列（含自身位置标记）
    pub position_in_parent: usize,   // 在父节点子元素中的索引
    pub parent_tag: String,
    pub parent_attrs: HashMap<String, String>,
}

impl ElementSnapshot {
    /// 从 Node 捕获快照
    /// 阶段 1：用 scraper::ElementRef 的树 API 拿上下文（绕过 Node 当前限制）
    /// 阶段 2：Node 重构后改为用 Node 的导航 API
    pub fn capture(node: &Node) -> Self;
}
```

### 1.1.3 SequenceMatcher（difflib 移植）

```rust
// src/parser/adaptive.rs
/// Python difflib.SequenceMatcher 的 Rust 移植。
/// 用于比较两个序列的相似度（文本、属性、标签序列等）。
///
/// `autojunk`: 对应 Python 的 autojunk 参数。true 时启用启发式垃圾检测——
/// 当某元素在 b 中出现次数超过 len(b)/100 + 3 时，将其视为"垃圾"，
/// 在 find_longest_match 中跳过。Python 默认 true，此处同样默认 true。
pub struct SequenceMatcher<'a, T: PartialEq> {
    a: &'a [T],
    b: &'a [T],
    autojunk: bool,
}

impl<'a, T: PartialEq> SequenceMatcher<'a, T> {
    pub fn new(a: &'a [T], b: &'a [T]) -> Self;

    /// 返回 0.0..1.0 的相似度比率（对应 Python 的 ratio()）
    /// 内部用 find_longest_match 递归求 LCS 块
    pub fn ratio(&self) -> f64;

    /// 找到 a[a1..a2] 与 b[b1..b2] 间的最长公共子串
    pub fn find_longest_match(&self, a1: usize, a2: usize, b1: usize, b2: usize) -> Match;
}

pub struct Match { pub a_start: usize, pub b_start: usize, pub size: usize }
```

**移植验证**：用 Python `difflib` 的官方测试用例做对照（同一段输入，ratio 相同）。

### 1.1.4 similarity 多维评分

总权重 8.0，归一化到 0..1。`DEFAULT_TOLERANCE = 0.5`（与 Python 一致）。

| 维度 | 权重 | 计算方式 |
|---|---|---|
| Tag 匹配 | 1.0 | 完全相等得 1.0 |
| 属性重叠 + 值相似度 | 2.0 | key Jaccard (0.5) + class 顺序相似度 (0.5) |
| 文本相似度 | 2.0 | `SequenceMatcher.ratio()`（字符级） |
| 祖先路径相似度 | 1.5 | `SequenceMatcher.ratio()`（标签序列级） |
| 兄弟标签序列相似度 | 1.0 | `SequenceMatcher.ratio()` |
| 父节点属性相似度 | 0.5 | key Jaccard |

```rust
fn similarity(node: &Node, saved: &ElementSnapshot) -> f64 {
    let mut score = 0.0;
    let mut max = 0.0;

    // 1. Tag 匹配（权重 1.0）
    max += 1.0;
    if node.tag() == saved.tag { score += 1.0; }

    // 2. 属性重叠 + 值相似度（权重 2.0）
    max += 2.0;
    let node_attrs = node.attrs();
    let key_overlap = saved.attrs.keys()
        .filter(|k| node_attrs.contains_key(*k)).count();
    let key_jaccard = key_overlap as f64 /
        (saved.attrs.len() + node_attrs.len() - key_overlap).max(1) as f64;
    let class_sim = match (node_attrs.get("class"), saved.attrs.get("class")) {
        (Some(a), Some(b)) => {
            let a_tokens: Vec<&str> = a.split_whitespace().collect();
            let b_tokens: Vec<&str> = b.split_whitespace().collect();
            SequenceMatcher::new(&a_tokens, &b_tokens).ratio()
        }
        _ => 0.0
    };
    score += 2.0 * (0.5 * key_jaccard + 0.5 * class_sim);

    // 3. 文本相似度（权重 2.0）
    max += 2.0;
    let node_text = node.text();
    let text_ratio = SequenceMatcher::new(
        node_text.chars().collect::<Vec<_>>().as_slice(),
        saved.text_preview.chars().collect::<Vec<_>>().as_slice(),
    ).ratio();
    score += 2.0 * text_ratio;

    // 4. 祖先路径相似度（权重 1.5）
    max += 1.5;
    let node_path = ancestor_path(node);
    let path_ratio = SequenceMatcher::new(
        node_path.as_slice(),
        saved.ancestor_path.as_slice(),
    ).ratio();
    score += 1.5 * path_ratio;

    // 5. 兄弟标签序列相似度（权重 1.0）
    max += 1.0;
    let node_siblings = sibling_tags(node);
    let sib_ratio = SequenceMatcher::new(
        node_siblings.as_slice(),
        saved.sibling_tags.as_slice(),
    ).ratio();
    score += 1.0 * sib_ratio;

    // 6. 父节点属性相似度（权重 0.5）
    max += 0.5;
    // 略

    score / max
}
```

### 1.1.5 css_adaptive API

```rust
// src/parser/adaptive.rs
use crate::storage::Store;

/// 默认重定位容差（与 Python Scrapling 对齐）
pub const DEFAULT_TOLERANCE: f64 = 0.5;

impl Node {
    /// 用 CSS 选择器查找；若失败则基于快照自适应重定位。
    ///
    /// - `key`: 元素的稳定标识符（用户自定义，如 "product-card"）
    /// - `store`: SQLite 存储
    /// - `auto_save`: 找到元素后是否刷新快照
    /// - `tolerance`: 相似度阈值
    pub fn css_adaptive(
        &self,
        selector: &str,
        key: &str,
        store: &Store,
        auto_save: bool,
        tolerance: f64,
    ) -> Result<NodeList>;
}

/// 无 CSS 选择器的纯重定位（基于已保存快照找最佳匹配）
pub fn relocate(
    html: &str,
    base_url: &str,
    key: &str,
    store: &Store,
    tolerance: f64,
) -> Result<Option<Node>>;
```

## 1.2 Spider 真并发（buffer_unordered 流式）

### 1.2.1 Engine 重构

当前 `Engine::run` 是 `while let Some(req) = sched.pop()` 串行。重构为 stream 管道：

```rust
// src/crawl/mod.rs
use futures::stream::{self, StreamExt};
use tokio::sync::Mutex;

pub struct Engine<S: Spider> {
    spider: S,
    config: EngineConfig,
}

pub struct EngineConfig {
    pub max_pages: usize,
    pub max_concurrent: usize,              // 默认 8，对应 spider.concurrent_requests()
    pub checkpoint_store: Option<Arc<Store>>,  // None = 不持久化
    pub checkpoint_interval: usize,            // 每 N 页保存，默认 100
}
```

### 1.2.2 并发管道设计

```
┌─────────────┐     ┌──────────────────────┐     ┌────────────────┐
│ Scheduler   │────▶│  buffer_unordered(N) │────▶│  parse + items │
│ (Mutex<Heap>)│    │  (并发请求 + retry)   │     │  (串行回调)    │
└─────────────┘     └──────────────────────┘     └────────────────┘
      ▲                     │                            │
      │                     ▼                            │
      │             ┌───────────────┐                    │
      └─────────────│  follow reqs  │◀───────────────────┘
                    │  push back     │
                    └───────────────┘
```

关键挑战：follow requests 要回灌到 scheduler，但 buffer_unordered 是无回灌的。解法用 channel。

**注意**：`state.client` 字段类型在阶段 1 是 `reqwest::Client`，阶段 2 切换到 `wreq::Client`。由于 `fetch::Client` 封装了底层类型（见 2.1.2），Engine 内部只持有 `fetch::Client`，不直接接触 reqwest/wreq 类型——阶段 2 切换对 Engine 透明。

```rust
// 用 channel 把 follow requests 回灌
let (follow_tx, follow_rx) = tokio::sync::mpsc::unbounded_channel();

// 主循环：从 scheduler + follow_rx 合并取请求，喂给 buffer_unordered
let stream = stream::unfold(state, move |mut state| async move {
    // 1. 先排空 follow channel
    while let Ok(req) = follow_rx.try_recv() {
        state.scheduler.push(req).await;
    }
    // 2. 从 scheduler 取下一个
    let req = state.scheduler.pop().await?;
    // 3. 发起请求
    let resp = fetch_with_retry(&state.client, &req, &state.spider).await;
    // 4. 调用 parse，拿 items + follow
    let (items, follow) = state.spider.parse(resp).await;
    // 5. follow 回灌
    for f in follow { let _ = follow_tx.send(f); }
    // 6. per-domain throttle
    throttle(&state.domain_semaphores, &req.url).await;
    Some((items, state))
}).buffer_unordered(config.max_concurrent);
```

### 1.2.3 per-domain throttle

用 `HashMap<domain, Arc<Semaphore>>`，每个域名一个信号量，并发上限可配置（默认 = 全局并发数）。

### 1.2.4 取消与优雅关闭

- Ctrl+C → `tokio::signal::ctrl_c()` future 与主循环 `select!`
- 收到信号后：停止从 scheduler 取新请求，等当前在飞的请求完成
- 触发 checkpoint 保存（见 1.3）

## 1.3 Spider checkpoint（SQLite 持久化）

### 1.3.1 CrawlState 结构

```rust
// src/crawl/mod.rs
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlState {
    pub spider_name: String,
    pub pending_urls: Vec<SpiderRequest>,      // scheduler 里的待爬队列
    pub seen_urls: HashSet<String>,            // 已访问去重集合
    pub stats: CrawlStats,
    pub saved_at: chrono::DateTime<chrono::Utc>,
}
```

### 1.3.2 Store API（复用 storage 层）

checkpoint 数据本身是结构化的 Rust 类型，blob 用 bincode 序列化最紧凑快速；用户查询入口仍走 SQLite（SELECT WHERE spider_name = ?），满足"可查询"需求。

```rust
// src/storage/mod.rs
impl Store {
    pub fn save_checkpoint(&self, state: &CrawlState) -> Result<()> {
        let blob = bincode::serialize(state)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO crawl_checkpoints (spider_name, state, saved_at) VALUES (?1, ?2, ?3)",
            params![state.spider_name, blob, state.saved_at.timestamp()],
        )?;
        Ok(())
    }

    pub fn load_checkpoint(&self, spider_name: &str) -> Result<Option<CrawlState>> {
        // SELECT state FROM crawl_checkpoints WHERE spider_name = ?1
        // bincode::deserialize
    }

    pub fn delete_checkpoint(&self, spider_name: &str) -> Result<()>;
}
```

### 1.3.3 Engine 集成

```rust
// src/crawl/mod.rs
impl<S: Spider> Engine<S> {
    pub fn with_checkpoint(spider: S, store: Arc<Store>) -> Self;

    // 启动时：尝试 load_checkpoint(spider.name())
    //   - 有 → 恢复 pending_urls 和 seen_urls，打印"恢复 N 个待爬 URL"
    //   - 无 → 从 start_urls 开始

    // 运行中：
    //   - 每 N 个页面保存一次（可配置，默认 100）
    //   - Ctrl+C 时保存
    //   - 正常结束或异常时保存

    // 完成后：
    //   - delete_checkpoint(spider.name())  // 清理已完成任务的 checkpoint
}
```

## 1.4 阶段 1 测试策略

| 测试项 | 方法 |
|---|---|
| SequenceMatcher 正确性 | 用 Python `difflib` 的官方测试用例做对照（同输入，ratio 相同） |
| adaptive 重定位 | 构造"改版前 HTML → 保存快照 → 改版后 HTML → 重定位"场景，断言找到的元素与 Python Scrapling 一致 |
| Spider 并发 | 用 `httptest` mock 多个 URL，断言并发数不超过 `max_concurrent` |
| Spider checkpoint | 启动爬取 → Ctrl+C → 重启 → 断言从上次中断处继续 |
| per-domain throttle | 同一域名并发不超过阈值 |

## 1.5 阶段 1 不做的事（边界）

- Node 内部结构重构（留阶段 2）—— 但 `ElementSnapshot::capture` 用 scraper::ElementRef 的树 API 拿上下文，绕过 Node 限制
- XPath 真实现（留阶段 2）
- wreq 替换（留阶段 2）—— 阶段 1 仍用 reqwest
- 流式对外 API（阶段 1 内部用 stream，但对外仍是 `run()` 收集所有 items）
- MCP / Session / 广告拦截 / DoH（留阶段 3/4）

---

# 阶段 2：P1 解析与 Fetch 增强

## 2.1 wreq 替换 reqwest

### 2.1.1 替换范围

| 模块 | 当前 | 替换后 |
|---|---|---|
| `src/fetch/mod.rs` | `reqwest::Client` | `wreq::Client` |
| `src/fetch/proxy.rs` | `reqwest::Proxy` | `wreq::Proxy` |
| `Cargo.toml` | `reqwest = { version = "0.12", features = ["rustls-tls"], default-features = false }` | `wreq = "5"` + `wreq-util = "2"` |

**关键约束**：wreq 基于 BoringSSL，不能与 openssl-sys 共存。wisp 当前全栈用 rustls，无 openssl 依赖，切换安全。

### 2.1.2 Client API 保持兼容

```rust
// src/fetch/mod.rs
pub struct Client {
    http: wreq::Client,   // 内部类型换了
    config: Config,
}

// 公共 API 签名完全不变
impl Client {
    pub fn builder() -> ClientBuilder;
    pub async fn get(&self, url: &str) -> Result<Response>;
    pub async fn post(&self, url: &str, body: Option<&str>, json: Option<&Value>) -> Result<Response>;
    // ...
}
```

### 2.1.3 新增 TLS 指纹模拟配置

```rust
// src/fetch/mod.rs
use wreq_util::Emulation;

#[derive(Debug, Clone)]
pub struct Config {
    pub timeout: Duration,
    pub user_agent: Option<String>,
    pub headers: HashMap<String, String>,
    pub proxy: Option<String>,
    pub max_redirects: usize,
    // 新增
    pub emulation: Option<Emulation>,        // 浏览器设备模拟
    pub header_order: Option<Vec<String>>,   // 自定义 header 顺序
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // ...原字段
            // 默认 Chrome 136 指纹（覆盖最广）
            emulation: Some(Emulation::Chrome136),
            header_order: None,
        }
    }
}

impl ClientBuilder {
    /// 指定浏览器模拟（Chrome/Firefox/Safari）
    pub fn emulation(mut self, emu: Emulation) -> Self {
        self.config.emulation = Some(emu);
        self
    }

    /// 不做指纹模拟（纯 reqwest 行为，用于调试）
    pub fn no_emulation(mut self) -> Self {
        self.config.emulation = None;
        self
    }
}
```

### 2.1.4 构建依赖说明

wreq 编译需 BoringSSL 依赖。

**Windows（用户当前环境）**：
- `cmake`（已常见）
- `perl`（Strawberry Perl 推荐）
- `nasm`
- `libclang`（LLVM 安装时勾选）

在 README 增加"从源码构建"章节说明。

**vendored 模式**：用 `boring-sys` 的 `vendored` feature 自带 BoringSSL 源码，避免系统依赖问题。

## 2.2 sxd-xpath 集成（懒解析方案）

### 2.2.1 Node 内部结构重构

当前 Node 内部存 `scraper::Html`，无法支持完整的 DOM 导航。重构为 `Arc<Document>` 共享所有权：

```rust
// src/parser/mod.rs
use scraper::{Html, ElementRef};
use sxd_document::Package;
use std::sync::Arc;
use std::cell::OnceCell;

pub struct Document {
    html: Arc<Html>,
    sxd: OnceCell<Package>,  // 懒加载
}

#[derive(Clone)]
pub struct Node {
    doc: Arc<Document>,
    // 在 html 树中的节点 id（scraper::Node::Element 的 id）
    node_id: scraper::node::NodeId,
    // 懒加载的 sxd 对应节点
    sxd_element: OnceCell<sxd_document::dom::Element<'static>>,  // 'static 因 Package 由 Arc 持有
}
```

这样 `Node` 是 `'static` 的、可 Clone、API 向后兼容。代价是每次 `from_html` 包一层 `Arc`（成本极低）。

### 2.2.2 XPath 懒解析流程

```rust
impl Node {
    pub fn xpath(&self, expr: &str) -> NodeList {
        // 快速路径：简单 XPath 转 CSS（覆盖 80% 常见用法）
        if let Some(css) = xpath_to_css(expr) {
            return self.select(&css);
        }
        // 慢路径：完整 sxd-xpath 查询
        self.xpath_full(expr)
    }

    fn xpath_full(&self, expr: &str) -> NodeList {
        // 1. 懒加载 sxd-document
        let package = self.doc.sxd.get_or_init(|| {
            build_sxd_from_html(&self.doc.html)
        });
        let document = package.as_document();

        // 2. 懒加载当前节点对应的 sxd element
        let sxd_el = self.sxd_element.get_or_init(|| {
            locate_in_sxd(document, self).unwrap_or(document.root())
        });

        // 3. 执行 xpath
        let xpath = sxd_xpath::parse_xpath(expr)
            .map_err(|e| WispError::ParseError(e.to_string()))?;
        let value = xpath.evaluate(document, *sxd_el)?;

        // 4. 结果转回 NodeList（在 scraper 树里找对应节点）
        match value {
            sxd_xpath::Value::Nodeset(ns) => {
                ns.iter()
                    .filter_map(|n| find_in_scraper(&self.doc.html, n))
                    .collect()
            }
            _ => NodeList::empty(),
        }
    }
}
```

### 2.2.3 HTML5 容错处理

sxd_document 的 parser 是 XML 解析器，对 HTML5 容错弱。解法：先用 html5ever（scraper 内部）解析为规范化的 HTML 字符串，再喂给 sxd_document：

```rust
fn build_sxd_from_html(html: &str) -> Package {
    // 1. 用 scraper 解析（html5ever 容错）
    let parsed = Html::parse_document(html);
    let clean_html = parsed.html();  // 规范化后的 HTML 字符串

    // 2. 喂给 sxd_document
    // sxd_document::parser 对 HTML 的已知问题：
    //   - <br>/<img> 等空标签需要自闭合
    //   - <script>/<style> 内容会被当文本
    // html5ever 输出的 clean_html 已经处理了这些
    sxd_document::parser::parse(&clean_html)
        .unwrap_or_else(|_| Package::new())
}
```

### 2.2.4 xpath_to_css 保留为快速路径

当前 `src/parser/mod.rs` 的 `xpath_to_css` 转换器保留，覆盖简单 XPath（`//tag`、`//tag[@class='x']` 等 80% 常见用法），不支持的走完整 sxd-xpath。

## 2.3 DOM 导航重构

### 2.3.1 真实实现所有导航方法

当前 `parent/next_sibling/prev_sibling` 都返回 None，`matches()` 永远 false。基于 2.2.1 的 `Arc<Document>` 重构后，需新增/真实实现以下方法：

| 方法 | 当前状态 | 阶段 2 动作 |
|---|---|---|
| `parent()` | 返回 None | 真实实现 |
| `children()` | 未实现 | 新增 |
| `next_sibling()` / `prev_sibling()` | 返回 None | 真实实现 |
| `ancestors()` | 不存在 | **新增**（迭代器，从父到根） |
| `matches(css)` | 永远 false | 真实实现 |

`ancestors()` 是 `ElementSnapshot::capture`（2.3.2）依赖的关键方法，返回从父节点到文档根的迭代器。

```rust
impl Node {
    pub fn parent(&self) -> Option<Node> {
        let element = self.element_ref()?;
        element.parent().and_then(|p| {
            if p.value().is_element() {
                Some(Node::from_element_ref(self.doc.clone(), p))
            } else {
                None
            }
        })
    }

    pub fn children(&self) -> NodeList {
        let element = match self.element_ref() { Some(e) => e, None => return NodeList::empty() };
        let nodes: Vec<Node> = element.children()
            .filter(|c| c.value().is_element())
            .map(|c| Node::from_element_ref(self.doc.clone(), c))
            .collect();
        NodeList::new(nodes)
    }

    pub fn next_sibling(&self) -> Option<Node> {
        let element = self.element_ref()?;
        let mut sib = element.next_sibling();
        while let Some(s) = sib {
            if s.value().is_element() {
                return Some(Node::from_element_ref(self.doc.clone(), s));
            }
            sib = s.next_sibling();
        }
        None
    }

    pub fn prev_sibling(&self) -> Option<Node> {
        // 同上，prev_sibling()
    }

    pub fn matches(&self, css: &str) -> bool {
        let selector = match CssSelector::parse(css) { Ok(s) => s, Err(_) => return false };
        // 关键：matches() 需要 ancestor 上下文，scraper 的 Element::value() 有 matches()
        self.element_ref()
            .map(|e| e.value().matches(&selector))
            .unwrap_or(false)
    }
}
```

### 2.3.2 ElementSnapshot::capture 升级

阶段 1 的 capture 用 scraper::ElementRef 临时拿上下文；阶段 2 Node 重构后，capture 改为用 Node 的导航 API：

```rust
impl ElementSnapshot {
    pub fn capture(node: &Node) -> Self {
        let ancestor_path = node.ancestors()
            .filter_map(|n| {
                let tag = n.tag();
                let class = n.attr("class").unwrap_or_default();
                if class.is_empty() {
                    Some(tag)
                } else {
                    Some(format!("{}.{}", tag, class.split_whitespace().next()?))
                }
            })
            .collect::<Vec<_>>()
            .into_iter().rev().collect();

        let sibling_tags = node.parent()
            .map(|p| p.children().iter().map(|c| c.tag()).collect())
            .unwrap_or_default();

        // ...
    }
}
```

阶段 1 的临时实现会被阶段 2 替换为这个干净版本。

## 2.4 阶段 2 测试策略

| 测试项 | 方法 |
|---|---|
| wreq TLS 指纹 | 请求 `https://tls.peet.ws/api/all`，断言 JA3/JA4 与 `Emulation::Chrome136` 一致 |
| wreq 兼容性 | 跑阶段 1 的所有 fetch 测试，断言行为不变 |
| XPath 基础 | `//div`、`//div[@class='x']`、`//a[contains(@href, 'example')]` 等 |
| XPath 复杂 | 轴：`//div/following-sibling::p`、谓词：`//ul/li[position()>2]` |
| XPath 容错 | 不规范 HTML（未闭合标签、嵌套错误）能正常解析 |
| DOM 导航 | 构造测试 HTML，断言 parent/children/sibling 正确 |
| matches() | `node.matches("div.foo")` 返回正确 bool |
| 性能对比 | xpath vs select(css) 同查询，断言 xpath 懒解析只在首次调用时有开销 |

## 2.5 阶段 2 不做的事

- 流式对外 API（留阶段 3）
- MCP / Session / 广告拦截 / DoH（留阶段 3/4）
- 把 adaptive 的 capture 重写提前到阶段 1.5——仍按阶段 1 临时实现，阶段 2 重构时一并替换

## 2.6 阶段 2 风险与缓解

| 风险 | 缓解 |
|---|---|
| wreq BoringSSL 在 Windows 编译失败 | 提供 `vendored` feature，用 `boring-sys` 的 vendored 模式自带 BoringSSL 源码 |
| sxd_document 的 HTML 容错差 | 用 html5ever 规范化后喂给 sxd，不直接喂原始 HTML |
| Node 生命周期重构引入 bug | 用 `Arc<Document>` 共享所有权，保留 `Node::from_html() -> Node` 无生命周期 API |
| scraper 与 sxd 双 DOM 不一致 | xpath 结果回查 scraper 树时用 tag+path 定位，找不到则跳过（不 panic） |

---

# 阶段 3：P2 工程化与 MCP

## 3.1 流式输出

### 3.1.1 Stream API 设计

阶段 1 的 buffer_unordered 是内部并发，对外仍是 `run()` 收集所有 items。阶段 3 把内部 stream 暴露给用户：

```rust
// src/crawl/mod.rs
use tokio_stream::Stream;

pub struct CrawlStream {
    inner: Pin<Box<dyn Stream<Item = CrawlEvent> + Send>>,
}

/// 爬取过程中的事件流
#[derive(Debug, Clone)]
pub enum CrawlEvent {
    /// 解析出一个 item
    Item(Value),
    /// 完成一页（含统计）
    PageScraped { url: String, stats: CrawlStats },
    /// 请求失败
    Error { url: String, error: String },
    /// 爬取结束
    Done(CrawlStats),
}

impl<S: Spider> Engine<S> {
    /// 流式运行：边爬边产出事件
    pub fn stream(self) -> CrawlStream {
        let (tx, rx) = tokio::sync::mpsc::channel(128);

        tokio::spawn(async move {
            let result = self.run_with_sender(tx.clone()).await;
            match result {
                Ok(stats) => { let _ = tx.send(CrawlEvent::Done(stats)).await; }
                Err(e) => { let _ = tx.send(CrawlEvent::Error {
                    url: "*".into(), error: e.to_string()
                }).await; }
            }
        });

        CrawlStream {
            inner: Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx))
        }
    }
}

impl CrawlStream {
    /// 仅消费 Item 事件（最常见的用法）
    pub fn items(self) -> impl Stream<Item = Value> {
        self.inner.filter_map(|e| async move {
            match e { CrawlEvent::Item(v) => Some(v), _ => None }
        })
    }

    /// 消费所有事件（调试/监控用）
    pub fn events(self) -> impl Stream<Item = CrawlEvent> {
        self.inner
    }
}
```

### 3.1.2 使用示例

```rust
// 流式消费
let mut stream = engine.stream().items();
while let Some(item) = stream.next().await {
    println!("抓到: {}", item);
    // 边抓边写文件/推下游/喂 LLM
}

// 监控模式
let mut events = engine.stream().events();
while let Some(event) = events.next().await {
    match event {
        CrawlEvent::PageScraped { url, stats } => {
            println!("[{}/{}] {}", stats.pages_crawled, stats.items_scraped, url);
        }
        CrawlEvent::Item(v) => { /* 存数据库 */ }
        CrawlEvent::Done(s) => { println!("完成: {:#?}", s); break; }
        _ => {}
    }
}
```

### 3.1.3 与阶段 1 run() 的关系

```rust
impl<S: Spider> Engine<S> {
    /// 旧 API 保持兼容（内部走 stream，收集所有 items）
    pub async fn run(self) -> Result<CrawlStats> {
        let mut stream = self.stream().events();
        let mut final_stats = None;
        while let Some(event) = stream.next().await {
            if let CrawlEvent::Done(s) = event { final_stats = Some(s); break; }
        }
        Ok(final_stats.unwrap_or_default())
    }
}
```

## 3.2 JSON / JSONL 导出

### 3.2.1 Items 集合

```rust
// src/crawl/mod.rs
use std::path::Path;

/// 爬取结果集合
pub struct Items {
    items: Vec<Value>,
}

impl Items {
    pub fn new(items: Vec<Value>) -> Self { Self { items } }
    pub fn len(&self) -> usize { self.items.len() }
    pub fn is_empty(&self) -> bool { self.items.is_empty() }
    pub fn iter(&self) -> impl Iterator<Item = &Value> { self.items.iter() }

    /// 导出为 JSON 字符串
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(&self.items)
            .map_err(|e| WispError::Serialize(e.to_string()))
    }

    /// 导出为 JSONL（每行一个 JSON 对象）
    pub fn to_jsonl(&self) -> Result<String> {
        let mut out = String::new();
        for item in &self.items {
            serde_json::to_string(item)
                .map_err(|e| WispError::Serialize(e.to_string()))?;
            out.push('\n');
        }
        Ok(out)
    }

    /// 写入 JSON 文件
    pub fn to_json_file(&self, path: &Path) -> Result<()> {
        let json = self.to_json()?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// 写入 JSONL 文件
    pub fn to_jsonl_file(&self, path: &Path) -> Result<()> {
        let jsonl = self.to_jsonl()?;
        std::fs::write(path, jsonl)?;
        Ok(())
    }

    /// 流式写入 JSONL（边爬边写，避免内存堆积）
    pub fn writer(path: &Path) -> Result<JsonlWriter> {
        Ok(JsonlWriter {
            file: std::fs::File::create(path)?,
        })
    }
}

/// 流式 JSONL 写入器
pub struct JsonlWriter {
    file: std::fs::File,
}

impl JsonlWriter {
    pub fn write(&mut self, item: &Value) -> Result<()> {
        use std::io::Write;
        let line = serde_json::to_string(item)
            .map_err(|e| WispError::Serialize(e.to_string()))?;
        writeln!(self.file, "{}", line)?;
        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        use std::io::Write;
        self.file.flush()?;
        Ok(())
    }
}
```

### 3.2.2 CrawlStats 增强

```rust
pub struct CrawlStats {
    pub items_scraped: usize,
    pub pages_crawled: usize,
    pub errors: usize,
    pub duration: Duration,
    // 新增
    pub bytes_downloaded: u64,
    pub avg_response_time: Duration,
    pub domain_counts: HashMap<String, usize>,  // 每域名页数
}

impl CrawlStats {
    /// 打印人类可读的统计摘要
    pub fn summary(&self) -> String {
        format!(
            "爬取完成: {} 页 / {} items / {} 错误 / 耗时 {:?} / {:.1} KB",
            self.pages_crawled, self.items_scraped, self.errors,
            self.duration, self.bytes_downloaded as f64 / 1024.0
        )
    }
}
```

**注意**：`CrawlStats` 只含统计数字，不含 items 数据本身。`Items` 集合由 `Engine::run()` 或 `CrawlStream` 单独产出（见 3.1.1 的 `CrawlEvent::Item`），二者职责分离。

### 3.2.3 与流式输出的配合

```rust
// 边爬边写 JSONL
let mut writer = Items::writer(Path::new("products.jsonl"))?;
let mut stream = engine.stream().items();
while let Some(item) = stream.next().await {
    writer.write(&item)?;
}
writer.flush()?;
```

## 3.3 MCP Server

### 3.3.1 架构

```
Claude / Cursor / 任意 MCP 客户端
        │
        │ stdio (JSON-RPC 2.0 over stdin/stdout)
        │
        ▼
┌──────────────────────────┐
│  wisp mcp serve          │
│  (bin/wisp.rs 子命令)    │
│                          │
│  ┌────────────────────┐  │
│  │ MCP 协议层          │  │
│  │ - tools/list       │  │
│  │ - tools/call       │  │
│  │ - resources/list   │  │
│  │ - prompts/list     │  │
│  └─────────┬──────────┘  │
│            │              │
│  ┌─────────▼──────────┐  │
│  │ Tool 实现           │  │
│  │ - fetch_page        │  │
│  │ - extract_css       │  │
│  │ - extract_xpath     │  │
│  │ - crawl_site        │  │
│  │ - adaptive_scrape   │  │
│  │ - stealth_fetch     │  │
│  └─────────┬──────────┘  │
│            │              │
│  ┌─────────▼──────────┐  │
│  │ wisp 库             │  │
│  │ (fetch/parser/crawl)│  │
│  └────────────────────┘  │
└──────────────────────────┘
```

### 3.3.2 MCP 工具定义

6 个工具覆盖核心场景：

| 工具名 | 用途 | 关键参数 |
|---|---|---|
| `fetch_page` | 抓取单个网页，返回 HTML | url, emulation, wait_selector |
| `extract_css` | CSS 选择器提取元素 | html, selector |
| `extract_xpath` | XPath 提取元素 | html, xpath |
| `crawl_site` | 爬取整个站点，返回 JSONL | start_urls, css_selector, max_pages, follow_pattern |
| `adaptive_scrape` | 自适应抓取（长期监控） | url, selector, key, db_path |
| `stealth_fetch` | 浏览器模式抓取（绕 CF Turnstile） | url, headless, human_mode |

```rust
// src/mcp/mod.rs
use serde::{Serialize, Deserialize};
use serde_json::{Value, json};

pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

pub const TOOLS: &[Tool] = &[
    Tool {
        name: "fetch_page",
        description: "抓取单个网页，返回 HTML 文本。支持 wreq TLS 指纹模拟绕过轻度反 bot。",
        input_schema: json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "目标 URL" },
                "emulation": {
                    "type": "string",
                    "enum": ["chrome", "firefox", "safari"],
                    "description": "浏览器指纹模拟，默认 chrome"
                },
                "wait_selector": {
                    "type": "string",
                    "description": "等待该 CSS 选择器出现后返回"
                }
            },
            "required": ["url"]
        }),
    },
    // ... 其余 5 个工具
];
```

### 3.3.3 JSON-RPC 协议实现

```rust
// src/mcp/mod.rs
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};

/// MCP server 主循环
pub async fn serve(store: Arc<Store>) -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let reader = BufReader::new(stdin);
    let mut lines = reader.lines();

    while let Some(line) = lines.next_line().await? {
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let id = request.get("id").cloned();

        // 区分 result（成功）与 error（失败）两种 JSON-RPC 响应
        let response: Value = match method {
            "initialize" => json!({
                "jsonrpc": "2.0", "id": id,
                "result": handle_initialize()
            }),
            "tools/list" => json!({
                "jsonrpc": "2.0", "id": id,
                "result": handle_tools_list()
            }),
            "tools/call" => match handle_tools_call(request, &store).await {
                Ok(result) => json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": result
                }),
                Err(e) => json!({
                    "jsonrpc": "2.0", "id": id,
                    "error": {
                        "code": -32603,  // Internal error
                        "message": e.to_string()
                    }
                }),
            },
            "resources/list" => json!({
                "jsonrpc": "2.0", "id": id,
                "result": {"resources": []}
            }),
            "prompts/list" => json!({
                "jsonrpc": "2.0", "id": id,
                "result": {"prompts": []}
            }),
            _ => json!({
                "jsonrpc": "2.0", "id": id,
                "error": {
                    "code": -32601,  // Method not found
                    "message": format!("unknown method: {}", method)
                }
            }),
        };

        let response_str = serde_json::to_string(&response)?;
        stdout.write_all(response_str.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }

    Ok(())
}

async fn handle_tools_call(request: Value, store: &Arc<Store>) -> Result<Value> {
    let params = request.get("params").unwrap();
    let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    let result = match name {
        "fetch_page" => tools::fetch_page(args).await,
        "extract_css" => tools::extract_css(args).await,
        "extract_xpath" => tools::extract_xpath(args).await,
        "crawl_site" => tools::crawl_site(args, store).await,
        "adaptive_scrape" => tools::adaptive_scrape(args, store).await,
        "stealth_fetch" => tools::stealth_fetch(args).await,
        _ => Err(WispError::McpUnknownTool(name.into())),
    }?;

    Ok(json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&result)?
        }]
    }))
}
```

### 3.3.4 CLI 集成

```rust
// src/bin/wisp.rs
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "wisp", version, about = "Lightweight undetected browser automation")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 抓取单个 URL
    Scrape {
        url: String,
        #[arg(long)]
        selector: Option<String>,
        #[arg(long, default_value = "json")]
        format: String,
    },
    /// 启动 MCP server（stdio）
    Mcp {
        #[command(subcommand)]
        cmd: McpCmd,
    },
}

#[derive(Subcommand)]
enum McpCmd {
    /// 启动 stdio MCP server
    Serve {
        #[arg(long, default_value = "./wisp.db")]
        db: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Scrape { url, selector, format } => { /* ... */ }
        Commands::Mcp { cmd } => match cmd {
            McpCmd::Serve { db } => {
                let store = Arc::new(Store::open(std::path::Path::new(&db))?);
                wisp::mcp::serve(store).await?;
            }
        }
    }
    Ok(())
}
```

### 3.3.5 Claude Desktop 配置示例

```json
{
  "mcpServers": {
    "wisp": {
      "command": "wisp",
      "args": ["mcp", "serve", "--db", "/path/to/wisp.db"]
    }
  }
}
```

## 3.4 阶段 3 测试策略

| 测试项 | 方法 |
|---|---|
| Stream 事件顺序 | 用 mock spider，断言 Item/PageScraped/Done 顺序正确 |
| Stream 背压 | 慢消费 + 快生产，断言 channel 不无限堆积 |
| JSON/JSONL 导出 | 构造 items，导出后用 `serde_json::from_str` 反序列化验证 |
| JsonlWriter 流式写 | 边爬边写 1000 条，断言文件行数 = items 数 |
| MCP tools/list | 启动 server，发 `{"method":"tools/list"}`，断言返回 6 个工具 |
| MCP tools/call | 调 fetch_page/extract_css，断言返回结构化 JSON |
| MCP 错误处理 | 未知工具名、参数缺失、URL 无效等场景 |
| Claude 集成 | 手动：配置 Claude Desktop，让它抓一个真实网页验证 |

## 3.5 阶段 3 不做的事

- HTTP/SSE 传输（仅 stdio，符合"内置于 CLI"决策）
- MCP resources/prompts 完整实现（仅返回空列表，专注 tools）
- Session 持久化（留阶段 4）
- 广告拦截 / DoH（留阶段 4）

## 3.6 阶段 3 风险与缓解

| 风险 | 缓解 |
|---|---|
| Stream 消费者慢导致爬虫背压 | 用 bounded channel(128)，满了就阻塞生产，自然限速 |
| MCP server 阻塞主线程 | 工具调用用 `tokio::spawn`，不阻塞 JSON-RPC 主循环 |
| crawl_site 工具超时（大站点） | 工具参数加 `max_pages` 默认 100，超时默认 5 分钟 |
| MCP 输出污染 stdout | 所有日志走 stderr，stdout 只输出 JSON-RPC |

---

# 阶段 4：P3 长尾能力

## 4.1 Session 持久化

### 4.1.1 Session 抽象

当前 `fetch::Client` 每次都是无状态请求，跨请求的 cookie/状态丢失。阶段 4 引入 Session 层：

```rust
// src/fetch/session.rs
use std::sync::Arc;
use crate::storage::Store;

/// 持久化会话：跨请求共享 cookie 和状态
pub struct Session {
    id: String,                    // 会话标识（用户自定义，如 "login-flow"）
    client: Client,                // 底层 HTTP 客户端（wreq，内置 cookie store）
    store: Arc<Store>,             // SQLite 持久化
    cookies: tokio::sync::RwLock<Vec<Cookie>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub expires: Option<i64>,      // Unix 时间戳
    pub secure: bool,
    pub http_only: bool,
}

impl Session {
    /// 创建或恢复会话
    pub async fn open(id: &str, store: Arc<Store>) -> Result<Self> {
        let client = Client::builder()
            .emulation(Emulation::Chrome136)
            .build()?;

        // 从 SQLite 恢复 cookies
        let cookies = store.load_cookies(id).unwrap_or_default();

        Ok(Self {
            id: id.to_string(),
            client,
            store,
            cookies: RwLock::new(cookies),
        })
    }

    /// 发请求（自动带 cookie，响应自动更新 cookie）
    pub async fn get(&self, url: &str) -> Result<Response> {
        // 1. 从 wreq client 的 cookie store 拿（wreq 内置 cookie store）
        // 2. 发请求
        let resp = self.client.get(url).await?;
        // 3. 从响应提取 Set-Cookie，更新内存 + 异步落盘
        self.update_cookies_from_response(&resp).await?;
        Ok(resp)
    }

    /// 显式保存到 SQLite
    pub async fn save(&self) -> Result<()> {
        let cookies = self.cookies.read().await;
        self.store.save_cookies(&self.id, &cookies)?;
        Ok(())
    }

    /// 导出为 Netscape cookie.txt 格式（与 curl/yt-dlp 互操作）
    pub fn to_netscape_format(&self) -> String {
        // # Netscape HTTP Cookie File
        // domain  flag  path  secure  expiration  name  value
    }

    /// 从 Netscape cookie.txt 导入
    pub fn from_netscape_format(&self, content: &str) -> Result<()>;

    /// 关闭会话（保存状态）
    pub async fn close(&self) -> Result<()> {
        self.save().await
    }
}
```

### 4.1.2 Session 类型对齐 Python Scrapling

Python Scrapling 有三种 Session：

| Python | wisp 对应 | 说明 |
|---|---|---|
| `FetcherSession` | `Session`（HTTP） | wreq cookie store + SQLite 持久化 |
| `StealthySession` | `StealthySession`（浏览器） | 复用 wisp 的 Browser/Page，cookie 落盘 |
| `DynamicSession` | `StealthySession`（浏览器，`dynamic: true`） | **复用 StealthySession 实现**，仅 `dynamic` 配置项差异（启用 JS 渲染等待） |

**说明**：wisp 当前的 Browser 已支持动态等待（wait_for_selector 等），Python 的 StealthySession 与 DynamicSession 在 wisp 中差异仅是配置参数（是否强制等 JS 渲染），不需要拆成两个类型。`StealthySession::open` 增加 `dynamic: bool` 参数控制。

```rust
// src/scraper/session.rs（浏览器会话，复用现有 Browser）
pub struct StealthySession {
    id: String,
    browser: Browser,              // wisp 现有的 CDP 浏览器
    page: Page,                    // 复用现有 Page
    store: Arc<Store>,
}

impl StealthySession {
    pub async fn open(id: &str, url: &str, store: Arc<Store>, headless: bool) -> Result<Self>;

    /// 在浏览器内执行抓取（带 cookie 上下文）
    pub async fn fetch(&self, url: &str) -> Result<ScrapeResponse>;

    /// 导出浏览器 cookie 到 SQLite
    pub async fn export_cookies(&self) -> Result<()>;

    /// 从 SQLite 恢复 cookie 到浏览器
    pub async fn import_cookies(&self) -> Result<()>;
}
```

### 4.1.3 Store API 扩展

```rust
// src/storage/mod.rs
impl Store {
    pub fn save_cookies(&self, session_id: &str, cookies: &[Cookie]) -> Result<()> {
        // 先删旧的，再批量插入
        self.conn.execute(
            "DELETE FROM session_cookies WHERE session_id = ?1",
            params![session_id],
        )?;
        for c in cookies {
            self.conn.execute(
                "INSERT OR REPLACE INTO session_cookies
                 (session_id, name, value, domain, path, expires, secure, http_only)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![session_id, c.name, c.value, c.domain, c.path, c.expires, c.secure, c.http_only],
            )?;
        }
        Ok(())
    }

    pub fn load_cookies(&self, session_id: &str) -> Result<Vec<Cookie>> {
        // SELECT name, value, domain, path, expires, secure, http_only
        // FROM session_cookies WHERE session_id = ?1
    }

    pub fn list_sessions(&self) -> Result<Vec<String>> {
        // SELECT DISTINCT session_id FROM session_cookies
    }

    pub fn delete_session(&self, session_id: &str) -> Result<()>;
}
```

## 4.2 开发模式 Replay（cache.rs 升级）

### 4.2.1 当前问题

`src/crawl/cache.rs` 当前是纯文件存储（hash 文件名），无 replay 逻辑，且 Spider Engine 没接入。

### 4.2.2 Replay 模式设计

```rust
// src/crawl/cache.rs（升级）
use std::path::PathBuf;
use crate::storage::Store;

pub struct ResponseCache {
    store: Arc<Store>,  // 改用 SQLite，替代文件 hash
}

impl ResponseCache {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    /// 获取缓存的响应
    pub fn get(&self, url: &str, method: &str) -> Result<Option<CachedResponse>> {
        self.store.load_cached_response(url, method)
    }

    /// 存储响应
    pub fn put(&self, url: &str, method: &str, resp: &CachedResponse) -> Result<()> {
        self.store.save_cached_response(url, method, resp)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub cached_at: i64,
}
```

### 4.2.3 Engine 集成 Replay 模式

```rust
// src/crawl/mod.rs
pub struct EngineConfig {
    // ...已有字段
    pub dev_mode: bool,                  // 开发模式：首次请求落盘，后续 replay
    pub cache_store: Option<Arc<Store>>,  // None = 不缓存
}

impl<S: Spider> Engine<S> {
    async fn fetch_page(&self, client: &Client, req: &SpiderRequest) -> Result<SpiderResponse> {
        // 开发模式：先查缓存
        if self.config.dev_mode {
            if let Some(cache) = &self.config.cache_store {
                if let Some(cached) = cache.get(&req.url, &req.method.to_string())? {
                    tracing::debug!("replay: {}", req.url);
                    return Ok(SpiderResponse::from_cached(cached, req.clone()));
                }
            }
        }

        // 正常请求
        let resp = /* 实际请求 */;

        // 开发模式：落盘
        if self.config.dev_mode {
            if let Some(cache) = &self.config.cache_store {
                cache.put(&req.url, &req.method.to_string(), &CachedResponse::from(&resp))?;
            }
        }

        Ok(resp)
    }
}
```

### 4.2.4 使用场景

```rust
// 开发模式：迭代 parse 逻辑时不重复请求
let engine = Engine::new(spider)
    .with_config(EngineConfig {
        dev_mode: true,
        cache_store: Some(Arc::new(store)),
        ..
    });

// 第一次运行：请求落盘
engine.run().await?;

// 修改 parse 逻辑后再次运行：直接 replay，不碰网络
engine.run().await?;
```

## 4.3 广告拦截

### 4.3.1 拦截策略

复用 wisp 现有的 CDP `Network.setBlockedURLs` 能力（browser 层已有），核心是维护广告域名列表：

```rust
// src/browser/adblock.rs
pub struct AdBlocker {
    blocked_domains: HashSet<String>,
}

impl AdBlocker {
    /// 加载内置广告域名列表（~3500 条，来源：Python Scrapling 使用的列表）
    pub fn with_builtin() -> Self {
        let domains: HashSet<String> = serde_json::from_str(BUILTIN_AD_DOMAINS).unwrap();
        Self { blocked_domains: domains }
    }

    /// 添加自定义域名
    pub fn add_domain(&mut self, domain: &str) {
        self.blocked_domains.insert(domain.to_string());
    }

    /// 从文件加载域名列表（用户自定义）
    pub fn load_from_file(path: &Path) -> Result<Self>;

    /// 生成 CDP 拦截模式（*://*.domain/*）
    pub fn to_url_patterns(&self) -> Vec<String> {
        self.blocked_domains.iter()
            .flat_map(|d| vec![
                format!("*://*.{}/*", d),
                format!("*://{}/*", d),
            ])
            .collect()
    }

    /// 判断 URL 是否该拦截
    pub fn should_block(&self, url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                // 检查 host 本身或其父域名是否在列表
                let mut current = host;
                loop {
                    if self.blocked_domains.contains(current) {
                        return true;
                    }
                    if let Some(idx) = current.find('.') {
                        current = &current[idx + 1..];
                    } else {
                        break;
                    }
                }
            }
        }
        false
    }
}

// 内置广告域名列表（从 Python Scrapling 仓库同步）
const BUILTIN_AD_DOMAINS: &str = include_str!("../data/ad_domains.json");
```

### 4.3.2 Browser 集成

```rust
// src/browser/launch.rs
pub struct LaunchOptions {
    // ...已有字段
    pub adblock: bool,                // 启用广告拦截
    pub blocked_domains: Vec<String>, // 额外拦截域名
}

impl Browser {
    pub async fn launch(opts: LaunchOptions) -> Result<Self> {
        // ...现有启动逻辑

        if opts.adblock {
            let blocker = AdBlocker::with_builtin();
            for d in &opts.blocked_domains {
                blocker.add_domain(d);
            }
            // 调用 CDP Network.setBlockedURLs
            page.call_method("Network.setBlockedURLs", json!({
                "urls": blocker.to_url_patterns()
            })).await?;
        }

        Ok(browser)
    }
}
```

## 4.4 DoH（DNS over HTTPS）

### 4.4.1 wreq API 已验证

经查 wreq 源码（`src/client.rs`），wreq 完整支持自定义 DNS resolver：

| API | 位置 | 用途 |
|---|---|---|
| `dns_resolver<R: IntoResolve>(self, resolver: R)` | `src/client.rs:1581` | 注入自定义 resolver |
| `resolve<D>(self, domain: D, addr: SocketAddr)` | `src/client.rs:1548` | 单域名 IP 覆盖 |
| `resolve_to_addrs<D, A>(self, domain, addrs)` | `src/client.rs:1564` | 单域名多 IP 覆盖 |

`Resolve` trait 定义在 `src/dns/resolve.rs`：

```rust
pub trait Resolve: Send + Sync {
    fn resolve(&self, name: Name) -> Resolving;
}
```

与 reqwest 的 API 完全一致——wreq 是 reqwest 的 fork，保留了 `dns_resolver` 方法。

### 4.4.2 DohResolver 设计

```rust
// src/fetch/doh.rs
use wreq::dns::{Name, Resolve, Resolving};
use std::net::SocketAddr;

/// DoH 解析器：通过 Cloudflare 1.1.1.1 的 DoH 端点解析域名
pub struct DohResolver {
    client: wreq::Client,
    doh_url: String,
}

impl DohResolver {
    pub fn new() -> Result<Self> {
        let client = wreq::Client::builder()
            .emulation(Emulation::Chrome136)
            .build()?;
        Ok(Self {
            client,
            doh_url: "https://cloudflare-dns.com/dns-query".to_string(),
        })
    }

    /// 自定义 DoH 端点（如 1.1.1.1 之外的 DoH 服务）
    pub fn with_endpoint(doh_url: String) -> Result<Self>;
}

impl Resolve for DohResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let client = self.client.clone();
        let doh_url = self.doh_url.clone();
        let host = name.as_str().to_string();

        Box::pin(async move {
            let url = format!("{}?name={}&type=A", doh_url, host);
            let resp = client.get(&url)
                .header("Accept", "application/dns-json")
                .await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
            let dns_resp: DohResponse = resp.json().await
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;

            let addrs: Vec<SocketAddr> = dns_resp.answers.iter()
                .filter(|a| a.type_ == 1)
                .filter_map(|a| a.data.parse::<std::net::IpAddr>().ok())
                .map(|ip| SocketAddr::new(ip, 0))  // 端口 0 由 wreq 按 scheme 填充
                .collect();

            Ok(Box::new(addrs.into_iter()) as Box<dyn Iterator<Item = SocketAddr> + Send>)
        })
    }
}

#[derive(Deserialize)]
struct DohResponse {
    status: u16,
    answer: Vec<DohAnswer>,
}

#[derive(Deserialize)]
struct DohAnswer {
    name: String,
    type_: u16,
    data: String,
}
```

### 4.4.3 使用

```rust
let doh = DohResolver::new()?;
let client = wreq::Client::builder()
    .dns_resolver(doh)              // 直接注入
    .emulation(Emulation::Chrome136)
    .build()?;
```

### 4.4.4 Fallback 策略

DoH 解析可能失败（网络中断、DoH 端点不可达、响应格式异常）。`DohResolver::resolve` 在失败时 fallback 到系统 resolver（`wreq::dns::GaiResolver`）：

```rust
impl Resolve for DohResolver {
    fn resolve(&self, name: Name) -> Resolving {
        // ... DoH 请求逻辑 ...
        // 失败时 fallback：
        let fallback = wreq::dns::GaiResolver::default();
        Box::pin(async move {
            match doh_lookup(&client, &doh_url, &host).await {
                Ok(addrs) if !addrs.is_empty() => Ok(Box::new(addrs.into_iter())),
                _ => {
                    tracing::warn!("DoH 失败，fallback 到系统 resolver: {}", host);
                    fallback.resolve(name).await
                }
            }
        })
    }
}
```

这保证 DoH 不可用时不影响爬虫主流程。

## 4.5 阶段 4 测试策略

| 测试项 | 方法 |
|---|---|
| Session cookie 持久化 | 登录站点 → close → 重新 open → 断言 cookie 恢复 |
| Netscape 格式互操作 | 导出后用 curl `--cookie` 加载验证 |
| Replay 模式 | 首次运行落盘 → 修改 parse → 二次运行断言无网络请求（用 mock server 计数） |
| 广告拦截 | 加载含广告的页面，断言广告请求被 block（CDP Network.requestWillBeSent 监听） |
| DoH 解析 | 解析 `google.com`，断言返回有效 IP；对比系统 resolver 结果 |
| DoH 防泄漏 | 抓包验证 DNS 查询走 HTTPS 而非 53 端口 |

## 4.6 阶段 4 不做的事

- 自建 DoH server（仅用 Cloudflare 公共端点）
- 广告域名列表自动更新（内置静态列表，用户可手动替换 `data/ad_domains.json`）
- Session 跨进程同步（同一进程内多 Session 实例通过 SQLite 共享，不做分布式锁）

## 4.7 阶段 4 风险与缓解

| 风险 | 缓解 |
|---|---|
| 广告域名列表过时 | 提供 `AdBlocker::load_from_file()` 让用户自定义 |
| Replay 缓存膨胀 | 加 TTL 字段，`cache.get()` 检查 `cached_at`，超期失效 |
| Session cookie 与 wreq 内置 cookie store 冲突 | 明确边界：wreq cookie store 管内存，Session.store 管持久化，二者通过 `save()`/`import()` 同步 |
| DoH 解析失败 | fallback 到系统 resolver（`GaiResolver`） |

---

# 实施顺序总览

| 阶段 | 内容 | 交付价值 |
|---|---|---|
| 阶段 1（P0 硬伤） | adaptive 完整移植 + Spider buffer_unordered 并发 + SQLite checkpoint | 爬虫框架"能用"——自适应 + 可恢复 |
| 阶段 2（P1 解析/fetch 增强） | wreq 替换 reqwest + sxd-xpath 懒解析 + DOM 导航重构 + matches() 真实现 | 解析能力对齐 + HTTP 反检测 |
| 阶段 3（P2 工程化） | 流式输出 + JSON/JSONL 导出 + MCP server（stdio） | AI agent 可调用 + 数据导出 |
| 阶段 4（P3 长尾） | Session SQLite 持久化 + cache.rs 升级为 replay + 广告拦截 + DoH | 长期爬取 + 反检测加固 |

每阶段独立可测、可发布；P0 先修硬伤；符合 rust_scrapling 的实现顺序（parser→fetch→spider→集成）；阶段 2 的 wreq 替换耦合风险通过"阶段 1 先不依赖 wreq 特性、只用 reqwest 基础接口"规避。
