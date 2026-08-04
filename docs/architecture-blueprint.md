# 架构重构蓝图

> 目标：消除 crate 间的逆依赖与循环依赖隐患，让每个 crate 职责单一、分层单向。
> 状态：待审阅（Draft）

## 1. 现状依赖图

```
core (纯 DTO/error/config: Request/Response/FetchMode/CrawlRequest)
│
├── http (transport: Client/Config/DomainBlocker)      → core
├── browser (CDP/page/pool: BrowserPool/Page)          → core
├── parser (HTML/Node: Document/ResponseExt)           → core
├── proxy (pool)                                        → 无 wisp 依赖
├── storage (Store: File/Memory/SQLite)                → core
│
├── stealth (CF/turnstile)                             → core + browser
│
├── fetcher (FetchClient + CookieJar + strategy)       → core + http + browser + stealth
│
├── crawl (Engine/Spider)                              → core + fetcher + parser + storage + proxy
│
└── mcp (Tools)                                        → core + crawl + fetcher + storage + parser
```

## 2. 职责错位与循环依赖隐患

### 2.1 CookieJar trait 归属错误（逆依赖）
- 位置：`crates/fetcher/src/cookie/mod.rs`
- 问题：`CookieJar` trait 定义在 fetcher，但它的实现者 `BrowserCookieJar`（纯浏览器领域，走 CDP）和消费者 `StealthStrategy` 都在浏览器领域。低层（browser）使用高层（fetcher）的 trait，是逆依赖。
- 影响：browser 领域逻辑无法脱离 fetcher 独立；若想迁移 StealthStrategy 到 browser 会立即成环。

### 2.2 stealth 与 browser 的依赖方向
- 位置：`crates/stealth/Cargo.toml`（依赖 browser）
- 问题：`wisp-stealth` 依赖 `wisp-browser`，但 stealth 的消费方（StealthStrategy 在 fetcher）又要用 stealth。若把 StealthStrategy 迁入 browser，则 browser → stealth → browser 成环。
- 影响：StealthStrategy 无法迁入 browser，除非把 `TurnstileConfig` 等纯参数下沉到 core。

### 2.3 fetcher 职责过载
- 位置：`crates/fetcher/src/client/fetch_client.rs`
- 问题：fetcher 同时承担 HTTP 关注点（连接池、代理、TLS 指纹）、浏览器策略编排（Dynamic/Stealth 分发）、Cookie 状态聚合。一个 crate 四个关注点。
- 影响：`FetchClient` 很臃肿，`fetch_browser` 混合资源生命周期 + 策略调用。

### 2.4 mcp 是薄壳但依赖过多底层
- 位置：`crates/mcp/src/tools/*`
- 问题：mcp 本应是"编排层"，却直接依赖 crawl/fetcher/parser/storage 四个 crate，每个工具都要自己实现 fetch/parse/storage 组合。
- 影响：mcp 工具与底层细节耦合，新增工具需要理解全部底层。

## 3. 目标分层结构

```
┌─────────────────────────────────────────────────────┐
│  core (契约层): DTO / 错误 / 配置 / 领域 trait        │
│  Request, Response, FetchMode, CrawlRequest,         │
│  Cookie, CookieJar (trait), Error, Text/Url/Encoding │
└─────────────────────────────────────────────────────┘
        │          │          │           │
        ▼          ▼          ▼           ▼
   ┌────────┐ ┌────────┐ ┌────────┐ ┌──────────┐
   │ http   │ │ browser│ │ parser │ │ storage  │
   │transport│ │ CDP/   │ │ HTML/  │ │ Store    │
   │Client  │ │ Page/  │ │ Node   │ │ File/Mem/│
   │        │ │ Pool   │ │        │ │ SQLite   │
   └────────┘ └────────┘ └────────┘ └──────────┘
        │          │
        ▼          ▼
   ┌──────────────┐
   │ stealth (CF) │      browser 提供策略 trait
   │ (依赖 browser)│      stealth 实现 CF 逻辑
   └──────────────┘
        │
        ▼
   ┌──────────────────────────────────────────────┐
   │  fetcher (分发层): FetchClient / 策略注册表    │
   │  只做 HTTP 分发 + 策略选择，不含浏览器内部细节  │
   └──────────────────────────────────────────────┘
        │
        ▼
   ┌──────────────────────────────────────────────┐
   │  crawl (编排层): Engine / Spider / 调度       │
   └──────────────────────────────────────────────┘
        │
        ▼
   ┌──────────────────────────────────────────────┐
   │  mcp (壳层): 工具注册 / 类型化 handler        │
   └──────────────────────────────────────────────┘
```

## 4. 分阶段迁移清单

### 阶段 A：CookieJar → core（消除逆依赖）
- 将 `Cookie` 结构体、`CookieJar` trait、`MockCookieJar` 从 `fetcher/src/cookie/mod.rs` 移到 `core/src/cookie.rs`（或 `core/src/types/cookie.rs`）。
- 依赖：`Cookie`/`CookieJar` 只依赖 `url` + `async_trait` + `serde`，core 已具备。
- 影响：fetcher/http/browser 的 `CookieJar` 引用改为 `wisp_core::cookie::*`。
- 验证：`cargo nextest run --workspace --all-features`。

### 阶段 B：BrowserCookieJar → browser
- 将 `BrowserCookieJar`（纯 CDP 实现）从 `fetcher/src/cookie/browser/` 移到 `browser/src/`。
- 依赖检查：它只依赖 `CdpSession`（browser）+ `CookieJar`（core）+ `url`，无循环。
- 影响：browser crate 变得自洽，不再依赖 fetcher 的 cookie 模块。

### 阶段 C：策略 trait 归属调整
- 将 `BrowserFetchStrategy` trait 移到 `browser`（或 core），作为浏览器领域契约。
- `DynamicStrategy`、`StealthStrategy` 留在 fetcher（依赖 config/cookie），但改用新的 trait 来源。
- 若后续要迁 StealthStrategy 到 browser，需先下沉 `TurnstileConfig` → core。

### 阶段 D：fetcher 职责瘦身
- `fetch_browser` 的浏览器资源生命周期（acquire/close/超时/blockedURLs）下沉到 browser 的 `BrowserPool` 或独立 `BrowserExecutor`。
- fetcher 只保留"选择策略 + 分发"。

### 阶段 E：mcp 薄壳化（可选）
- mcp 工具通过 crawl 的公开 API 组合，不在 mcp 内重复实现底层逻辑。

## 5. 风险与验证

- **每个阶段独立可编译、可测试**。阶段间存在依赖，须按 A→B→C→D 顺序推进。
- 每阶段完成后运行 `cargo nextest run --workspace --all-features` 与 `cargo test --doc`。
- 阶段 D 的 `fetch_browser` 下沉涉及浏览器生命周期，需确保 `browser_pool` 的 RAII 语义不变。

## 6. 待确认决策

1. 阶段 C：`BrowserFetchStrategy` 放 browser 还是 core？（建议 browser，浏览器领域契约）
2. 阶段 D：`fetch_browser` 生命周期下沉到 `BrowserPool` 方法还是独立 `BrowserExecutor` 模块？
3. 阶段 E：mcp 薄壳化是否本次纳入？