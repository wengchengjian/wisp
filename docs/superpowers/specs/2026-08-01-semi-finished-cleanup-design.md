# wisp 半成品与冗余修复设计

> 日期：2026-08-01
> 状态：已与用户逐项确认，待实现

## 目标

修复上轮 code review 发现的 18 项问题，重点消除“公开承诺但未生效”的半成品功能，清理死代码与冗余，并补齐并发/存储边界缺陷。总策略为补全为主；受底层库/浏览器能力限制的项允许显式降级或移除。

## 背景与范围

当前 `master`（`e2d08e0`）工作树干净，344 项库测试通过。问题分四组：

1. MCP 工具半成品：`stealth_fetch` 名不副实、`crawl_site.follow_pattern` 无效、`adaptive_scrape.db_path` 无效、MCP CLI `--db` 语义误导。
2. 抓取能力未接线：`DomainBlocker` 完全未生效、socks5 声称支持但不工作、浏览器代理认证静默降级。
3. 配置与边界：`header_order` 公开但无效、`max_concurrent(0)` 可能挂起、`FetchClient` 不优雅关闭 BrowserPool。
4. 存储/统计/增长：checkpoint 统计恢复不完整、三个无界增长点。
5. 死代码与命名：`http::proxy`、`UaRotator`、`CrawlState::from_stats`、`user_data_path`、过时 allow、`limit.rs` 命名。

## 已确认决策

- 全部 18 项一次处理，按独立提交组织。
- 底层不支持时允许降级/移除（`header_order` 移除、浏览器代理认证显式拒绝）。
- checkpoint 格式不做向后兼容。
- 存储默认保持 FileStore 轻量路径，元素快照序列化维持 serde_json，不引入 bincode。

## 实现设计

### 1. MCP 工具一致性

**共享 FetchClient**：MCP server 创建并持有 `Arc<FetchClient>`（含 BrowserPool），`stealth_fetch` 通过 `fetch_browser` + `StealthStrategy` 执行，复用挑战解决、CF cookie、人类行为链路。工具参数移除 `headless`/`human_mode`；MCP CLI 新增 `--headless`（默认 true）、`--human-mode`（默认 true）作为共享 FetchClient 的启动配置。schema 更新为仅 `url`。

**crawl_site 聚焦扩展**：工具读取 `follow_pattern`（可选正则）、`max_depth`（默认 0 = 不限制）、`allowed_domains`（默认不限）。`SimpleSpider` 增加字段并在 handle 中提取 `a[href]`：

- 过滤 scheme 为 http/https（复用 `resolve_href`）。
- 有 `follow_pattern` 时仅跟随匹配 URL 的链接。
- 生成 follow 请求时设置 `depth + 1`，超过 `max_depth` 不跟随。
- `allowed_domains` 非空时，不匹配域名的链接不跟随。
- `css_selector` 提取 items 逻辑不变。

**adaptive_scrape db_path**：工具读取 `db_path`，未传时回退 server 共享 store；传入时按路径创建 store：启用 `sqlite` feature 且路径非 `:memory:` 用 `SqliteStore::open`，否则用 `FileStore::with_dir`。快照读写沿用 `save_element/load_element`，序列化保持 serde_json。

**MCP CLI `--db`**：无 sqlite feature 时，输出明确提示“当前构建未启用 sqlite，使用 FileStore 目录”，不再静默忽略；启用 sqlite 时行为不变。

### 2. 抓取能力接线

**DomainBlocker**：

- HTTP：`FetchClient::fetch_http` 入口检查 `config.domain_blocker.should_block(&req.url)`，命中返回明确的 `WispError::Config`（含 URL），不发起请求。
- 浏览器：`FetchClient::fetch_browser` 在 acquire 后、strategy.fetch 前，若配置了 blocker，通过 CDP `Network.setBlockedURLs` 下发 `blocked_domains()` 的 `*://domain/*` 模式；Dynamic/Stealth 均生效。
- `DomainBlocker` 补充 `blocked_domains()` 访问器（返回带通配符的 URL 模式列表）。

**socks5**：

- `crates/http/Cargo.toml` 的 wreq 依赖增加 `socks` feature。
- `ClientBuilder::build` 按代理 scheme 分流：`socks5://`、`socks4://`、`socks4a://`、`socks5h://` 使用 `wreq::Proxy::http(proxy_url)`；其他使用 `wreq::Proxy::all(proxy_url)`。
- 删除 `crates/http/src/proxy.rs` 模块（`ParsedProxy`/`parse_proxies`/`to_proxy_url` 无生产引用），同步删除 re-export 与自身测试。
- README 与 Config 文档更新为“支持 http/https/socks4/socks5”。

**浏览器代理认证显式拒绝**：

- `launch/args.rs` 的 `push_proxy_arg` 改为返回 `Result<()>`：代理含 username/password 时返回 `WispError::Config("浏览器模式暂不支持代理认证")`。
- `Browser::launch` 在构建参数前调用并传播错误。
- `FetchClient::build_browser_pool` 改为 `Result<Option<Arc<BrowserPool>>>`：解析 `config.proxy` 的 userinfo，存在时返回同一 Config 错误。
- HTTP 模式（wreq URL 内嵌认证）不受影响。

### 3. 配置与边界

**header_order 移除**：删除 `Config.header_order` 字段、`ClientBuilder::header_order`、`tests/fetch_test.rs` 相关断言，文档同步。

**max_concurrent 校验**：`EngineBuilder::build` 校验 `max_concurrent == 0`，返回 `WispError::Config("max_concurrent must be > 0")`。

**FetchClient Drop**：实现 `Drop`，若 `browser_pool` 存在且 `tokio::runtime::Handle::try_current()` 成功，clone 池并在 `tokio::spawn` 中 `shutdown().await`；无 Runtime 时跳过（依赖 `Browser::Drop` kill 兜底）。显式 `browser_pool().shutdown()` 保留。

### 4. 存储与统计

**checkpoint 统计补齐**：`CrawlState` 增加字段：

- `status_codes: HashMap<u16, usize>`
- `blocked: usize`
- `retries: usize`
- `offsite: usize`
- `cache_hits: usize`

`persist_spider_checkpoint` 从 stats 快照填充；`restore_from` 恢复全部字段；不做旧格式兼容（旧 checkpoint 反序列化失败仅告警，行为同现状）。

**无界增长**：

- `ModeRuleEngine`：`auto_rules` 上限 1000，超限时停止学习新规则并告警一次。
- `cf_domain_locks`：容量上限 1024，超限时回退到全局共享锁并告警。
- `Scheduler.seen`：保持无界，仅保留现有容量告警（淘汰会破坏去重语义）。

### 5. 死代码与命名清理

- 删除 `crates/http/src/proxy.rs` 与 `http::proxy` re-export。
- 删除 `UaRotator`（`crates/http/src/ua.rs`）及其 re-export；`crawl::UaRotationMiddleware` 是唯一 UA 轮换实现。
- 删除 `CrawlState::from_stats`（无调用）。
- 删除 `Browser.user_data_path` 字段及 `#[allow(dead_code)]`。
- 移除 `strategy/extract.rs`、`strategy/event.rs` 的过时 `#[allow(dead_code)]`（函数已被 Dynamic/Stealth 使用）。
- `middleware/builtin/limit.rs` 重命名为 `cache_robots_delay.rs`，内容不变。
- `Fetcher::new` 非 browser 分支删除重复的 `Auto` 错误分支。
- `Fetcher::from_client` 补全：按 `FetchClientConfig` 自动构造 Dynamic/Stealth strategy（Stealth 复用 `cf_data_dir`/`cf_cookie_ttl` 创建 `CfCookieJar`），使该构造方式与 `Fetcher::new` 行为一致，不再留“fetch 时才报错”的陷阱。

## 接口变更

- 新增：`DomainBlocker::blocked_domains()`、MCP CLI `--headless`/`--human-mode`、`CrawlState` 五个统计字段。
- 变更：`FetchClient::build_browser_pool` 返回 `Result<Option<...>>`、`push_proxy_arg` 返回 `Result<()>`、`crawl_site` schema 扩展、`adaptive_scrape` db_path 生效、`stealth_fetch` schema 缩减。
- 移除：`Config.header_order`、`http::proxy` 模块、`UaRotator`、`CrawlState::from_stats`、`Browser.user_data_path`、`stealth_fetch.headless/human_mode` 参数。

## 错误处理

- 浏览器代理认证、`max_concurrent(0)`、blocker 命中均返回结构化 `WispError::Config`，消息明确。
- socks5/代理构建失败沿用现有 `NetworkError::Http` 包装。
- `FetchClient::Drop` 无 Runtime 时静默回退，不 panic。

## 测试计划

- MCP：`crawl_site` 跟随/深度/域名过滤、`adaptive_scrape` db_path 隔离、schema 与实现一致（含 stealth_fetch 参数变化）。
- HTTP blocker：配置后 `fetch_http` 返回 Config 错误且不发起请求。
- CDP blocker：单测断言 `blocked_domains()` 模式序列化；真实拦截测试保持 `#[ignore]`。
- socks5：代理 scheme 分流单测（socks5 走 `Proxy::http` 分支，构建成功）。
- 认证拒绝：`Browser::launch`/`build_browser_pool` 含认证代理返回 Config 错误。
- 边界：`max_concurrent(0)` 构建报错；`FetchClient::Drop` 在 Runtime 内触发池 shutdown（用 `is_launched` 断言，Chrome 测试 `#[ignore]`）。
- checkpoint：统计字段往返恢复测试（含 status_codes）。
- 增长上限：auto_rules 超限停止学习、cf_domain_locks 超限回退告警的单元测试。
- 清理项：编译与现有测试回归即可，删除的 API 无残留引用。

## 验证命令

```bash
cargo fmt --all -- --check
cargo check --workspace --all-features --all-targets
cargo check --workspace --no-default-features --all-targets
cargo clippy --all-targets --all-features --message-format=short
cargo test --workspace --all-features --lib --no-fail-fast
cargo test --workspace --all-features --tests --no-run --no-fail-fast
```
