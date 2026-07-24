# Wisp 项目全面代码审查缺陷报告

**审查日期**: 2026-07-24  
**审查范围**: 全项目（src/ 所有模块、Cargo.toml、tests/）  
**审查标准**: 行业最佳实践（正确性、安全性、架构、性能、错误处理、测试、文档）

---

## 一、正确性缺陷（Critical / High）

### D-001: DomainBlocker::should_block O(n) 线性扫描性能缺陷

- **严重度**: Critical
- **文件**: `src/http/block.rs` L72-84
- **描述**: `should_block()` 对 `blocked` HashSet 做全量迭代，每次检查执行 `format!(".{}", blocked)` 字符串拼接。启用广告拦截后约 200+ 域名，每次 URL 检查 O(n) 遍历 + 堆分配。在浏览器模式 CDP 拦截（每个子资源都调用）场景下是热路径。
- **复现路径**: 启用 `enable_ad_blocking()` 后在浏览器模式下加载含 50+ 子资源的页面。
- **修复建议**: 改为将 host 按 `.` 分段逐级查 HashSet（O(label_count)），或预构建 reversed-domain Trie。

### D-002: Response::css()/select_one() 每次调用重新解析 HTML

- **严重度**: High
- **文件**: `src/fetcher/response.rs` L281-299
- **描述**: `css()`、`select_one()`、`find_by_text()` 每次调用 `self.parse()` 重新解析整个 HTML 文档。Spider handler 中多次调用 `resp.css(".a")` + `resp.css(".b")` 会重复解析同一文档。
- **影响**: 大型 HTML 页面（>100KB）多次查询时 CPU 开销成倍增长。
- **修复建议**: 在 Response 内缓存 `OnceCell<Node>` 或提供 `resp.with_doc(|doc| ...)` 闭包 API。

### D-003: rand_suffix() 碰撞风险

- **严重度**: High
- **文件**: `src/utils/random.rs` L6-13
- **描述**: 仅使用 `subsec_nanos()`（0~999,999,999），同一秒内两次调用大概率碰撞。用于 Browser 临时目录名 `wisp-{pid}-{suffix}`，同进程并发 launch 多个 Browser 时目录冲突。
- **复现路径**: 并发调用 `BrowserPool::acquire()` 触发多个 Browser launch。
- **修复建议**: 使用 `rand` crate 生成随机 u64，或结合 PID + AtomicU64 计数器。

### D-004: ProxyPool::next() Sequential 模式 AtomicUsize 溢出

- **严重度**: Medium
- **文件**: `src/proxy.rs` L43-45
- **描述**: `fetch_add(1, Relaxed)` 在 `usize::MAX` 时 wrapping overflow。Debug 模式下会 panic（Rust debug 默认 overflow-checks=on）。
- **影响**: 长时间运行的爬虫理论上可在 debug 构建中触发 panic。
- **修复建议**: 使用 `fetch_add(1, Relaxed).wrapping_rem(len)` 或 `fetch_update` 取模。

### D-005: AutoscaledPool 使用 std::sync::Mutex 违反项目规范

- **严重度**: Medium
- **文件**: `src/crawl/runtime/autoscale.rs` L62-63
- **描述**: 项目约定使用 `parking_lot::Mutex` 避免 poison 传染（Cargo.toml 注释明确说明），但 `last_scale_up`/`last_scale_down` 使用 `std::sync::Mutex`，代码中用 `unwrap_or_else(|e| e.into_inner())` 手动处理 poison。
- **修复建议**: 替换为 `parking_lot::Mutex`，删除 poison 处理代码。

---

## 二、安全漏洞（Security）

### D-006: ProxyConfig 的 Debug 输出泄露凭据

- **严重度**: High
- **文件**: `src/config.rs` L31-36, `src/http/proxy.rs` L4-16
- **描述**: 两处 `ProxyConfig` 均 derive `Debug`，代理 URL 中的 `username:password` 在日志/调试输出中明文暴露。违反项目「敏感配置的 Debug 脱敏规范」。
- **复现路径**: `tracing::debug!("{:?}", proxy_config)` 或 panic 时自动输出。
- **修复建议**: 手动实现 `Debug`，将 password 替换为 `"***"`。

### D-007: MCP Server 无输入验证/速率限制

- **严重度**: Medium
- **文件**: `src/mcp/mod.rs` L99-168
- **描述**: MCP server 通过 stdio 接收 JSON-RPC 请求，`crawl_site` 工具接受任意 `start_urls` 和 `max_pages`（Engine 级上限 100000），无鉴权、无速率限制、无域名白名单。
- **影响**: 恶意 MCP 客户端可利用此 server 发起大规模爬取或 SSRF。
- **修复建议**: 对 `max_pages` 设 per-call 硬上限（如 1000），对 `start_urls` 做 scheme 校验（仅 http/https）。

### D-008: is_cloudflare_page 检测过于宽泛

- **严重度**: Low
- **文件**: `src/stealth/challenge.rs` L126-139
- **描述**: `body.includes('cloudflare')` 误判任何提及 Cloudflare 的正常页面。`[class*="cf-"]` 匹配范围过广（如 `cf-footer` 等自定义 class）。
- **影响**: 正常页面被误判为 CF 挑战页，触发不必要的等待/重试。
- **修复建议**: 收紧为 CF 挑战页特有标识（`cf-browser-verification`、`challenge-platform`、`cf-chl-`），移除泛化匹配。

---

## 三、架构设计问题（Architecture）

### D-009: ProxyConfig 类型重复定义

- **严重度**: Medium
- **文件**: `src/config.rs` L31-36 vs `src/http/proxy.rs` L4-16
- **描述**: 两个不同的 `ProxyConfig` 结构体，字段不同、用途不同但名称相同。违反项目编码规范「禁止在各子模块中重复定义核心类型」。
- **修复建议**: 统一为一个或重命名区分（如 `BrowserProxyConfig` / `HttpProxyConfig`）。

### D-010: http::Response 与 fetcher::Response 双重类型冗余转换

- **严重度**: Medium
- **文件**: `src/http/mod.rs` L338-368 vs `src/fetcher/response.rs` L183-204
- **描述**: `http::Response` 和 `fetcher::Response` 是独立类型，`fetch_page_inner`（engine.rs L541-555）中逐字段克隆转换。项目已在推进统一但尚未完成。
- **影响**: 每次 HTTP 响应经历完整字段克隆，大 body 时内存翻倍。
- **修复建议**: 完成 Request/Response 统一重构，消除 `http::Response` 中间类型。

### D-011: Engine run/run_stream 不可并发但无编译期保证

- **严重度**: Medium
- **文件**: `src/crawl/runner.rs` L69-92
- **描述**: 文档注释说明「不可并发调用」，但 `Engine` 是 `Clone + Send + Sync`，无编译期机制阻止。`control.reset()` 在每次 run 开头执行，并发调用会相互覆盖。
- **修复建议**: 使用 `Mutex<()>` 运行锁或 `&mut self` 签名提供编译期保证。

---

## 四、错误处理问题（Error Handling）

### D-012: Text::extract_regex 静默吞掉无效正则

- **严重度**: Low
- **文件**: `src/text.rs` L31-36
- **描述**: 无效正则表达式返回空 Vec，无日志或错误提示。用户传入错误 pattern 时得到空结果而不知原因。
- **修复建议**: 返回 `Result<Vec<String>>` 或至少 `tracing::warn!` 记录。

### D-013: build_headers_with 静默忽略无效 header

- **严重度**: Low
- **文件**: `src/http/mod.rs` L269-280
- **描述**: `HeaderName::from_bytes` 或 `HeaderValue::from_str` 失败时静默跳过，用户传入非法 header 名/值时请求缺少该 header 无提示。
- **修复建议**: 返回 Result 或 `tracing::warn!` 记录被跳过的 header。

### D-014: config_file.rs 的 ProxyConfig.strategy 使用 String 而非枚举

- **严重度**: Low
- **文件**: `src/config_file.rs` L63-70
- **描述**: `strategy` 字段为 `String`，接受任意值。拼写错误（如 `"sequntial"`）不报错，静默使用无效策略。
- **修复建议**: 使用 `RotationStrategy` 枚举 + `#[serde(rename_all = "lowercase")]`。

---

## 五、性能问题（Performance）

### D-015: DomainBlocker::should_block 中 format! 堆分配

- **严重度**: Medium
- **文件**: `src/http/block.rs` L79
- **描述**: 每次检查执行 `format!(".{}", blocked)` 创建临时 String。200 个 blocked 域名 × 每个子资源请求 = 大量短生命周期堆分配。
- **修复建议**: 预计算 `.domain` 后缀存储，或用 `host.len() > blocked.len() && host.ends_with(blocked) && host.as_bytes()[host.len()-blocked.len()-1] == b'.'`。

### D-016: UaRotator 每次构造克隆所有 UA 字符串

- **严重度**: Low
- **文件**: `src/http/ua.rs` L39-46
- **描述**: `desktop()`/`mobile()` 将 `&'static str` 常量全部 `.to_string()` 克隆到 `Vec<String>`。UA 是编译期常量无需拥有所有权。
- **修复建议**: 改为 `Vec<&'static str>` 或直接引用 `&'static [&'static str]`。

### D-017: fetch_page_inner 浏览器模式冗余 Method 类型转换

- **严重度**: Low
- **文件**: `src/crawl/engine.rs` L456-490
- **描述**: `crawl::Method` → `fetcher::Method` 逐 variant 匹配转换，但两者实际是同一类型（`pub use crate::fetcher::{Method, Request, Response}`）。历史遗留冗余代码。
- **修复建议**: 删除 match 转换，直接使用 `req.method`。

---

## 六、代码质量问题（Code Quality）

### D-018: MCP TOOLS 注释与实际数量不一致

- **严重度**: Low
- **文件**: `src/mcp/mod.rs` L22-24
- **描述**: 注释写「6 个工具覆盖核心场景」，实际只有 5 个工具。测试断言 `tools.len() == 5`。
- **修复建议**: 更新注释为「5 个工具」。

### D-019: Spider trait until 文档错别字

- **严重度**: Low
- **文件**: `src/crawl/mod.rs` L112
- **描述**: 「由引擎 max_pages 兖底」应为「兜底」。

### D-020: http::Response 的 content_type 字段私有但 body 公开

- **严重度**: Low
- **文件**: `src/http/mod.rs` L338-345
- **描述**: `content_type` 私有，`body`/`status`/`url`/`headers` 公开。API 一致性差。
- **修复建议**: 提供 `pub fn content_type(&self) -> &str` getter 或改为 pub。

### D-021: Browser::wait_for_devtools_url 参数 &PathBuf 不符合惯用法

- **严重度**: Low
- **文件**: `src/browser/mod.rs` L81
- **描述**: `&PathBuf` 参数触发 clippy::ptr_arg 警告，应使用 `&Path`。

### D-022: ClientBuilder 缺少 Default impl

- **严重度**: Low
- **文件**: `src/http/mod.rs` L60-62
- **描述**: `ClientBuilder::new()` 存在但未实现 `Default` trait，触发 clippy::new_without_default。

---

## 七、测试覆盖率问题（Testing）

### D-023: 浏览器模块无集成测试覆盖

- **严重度**: Medium
- **文件**: `src/browser/` (cdp.rs, page.rs, element.rs, patches.rs)
- **描述**: CDP 通信、JS 注入、元素操作等核心路径无自动化测试。仅 pool.rs 有不依赖 Chrome 的单元测试。
- **修复建议**: 添加 `#[ignore]` 标记的集成测试（需要 Chrome 环境）。

### D-024: Stealth 模块无测试

- **严重度**: Medium
- **文件**: `src/stealth/` (challenge.rs, turnstile.rs, human.rs)
- **描述**: CF 检测逻辑、Turnstile 解决逻辑、人类行为模拟均无测试覆盖。
- **修复建议**: 至少为 `ChallengeSolver::detect()` 的 JS 返回值解析添加 mock 测试。

### D-025: MCP tools.rs 无测试

- **严重度**: Low
- **文件**: `src/mcp/tools.rs`
- **描述**: 5 个工具实现（fetch_page, extract_css, crawl_site, adaptive_scrape, stealth_fetch）无单元测试。
- **修复建议**: 为 `extract_css` 添加纯逻辑测试（不依赖网络）。

### D-026: encoding.rs 无测试

- **严重度**: Medium
- **文件**: `src/http/encoding.rs`
- **描述**: 字符集检测是爬虫核心能力（GBK/Big5/EUC-JP/Shift_JIS 等），但无任何测试覆盖。
- **修复建议**: 添加各编码的 BOM/Content-Type/meta charset 检测测试。

---

## 八、依赖与构建配置（Dependencies）

### D-027: anyhow 依赖残留

- **严重度**: Medium
- **文件**: `Cargo.toml` L21
- **描述**: 项目规范明确要求「不要引入 anyhow」，但 Cargo.toml 仍依赖 `anyhow = "1"`。
- **修复建议**: 检查是否有代码使用，若无则移除依赖。

### D-028: wreq 固定为 RC 版本

- **严重度**: Low
- **文件**: `Cargo.toml` L29
- **描述**: `wreq = "=6.0.0-rc.29"` 精确锁定 RC 版本，API 不稳定。代码中多处注释提到「wreq 6.0.0-rc.29 未暴露 headers_order 方法」。
- **修复建议**: 跟踪 wreq 正式版发布，及时迁移。

### D-029: bincode 1.x 技术债务

- **严重度**: Low
- **文件**: `Cargo.toml` L37, `src/fetcher/response.rs` L18-31
- **描述**: bincode 1.x 不支持 `deserialize_any`，导致 `meta_serde` 需要自定义序列化模块绕过。注释说明「3.0.0 有编译问题」。
- **修复建议**: 跟踪 bincode 3.x 修复，或迁移到 postcard/rmp-serde。

---

## 九、文档完整性（Documentation）

### D-030: 公共 API 缺少 # Errors / # Panics 文档

- **严重度**: Low
- **文件**: 全项目公共函数
- **描述**: `Fetcher::get`、`Engine::run`、`Browser::launch`、`SpiderBuilder::build`（会 panic）等公共 API 未按 Rust API Guidelines 包含 `# Errors`/`# Panics` 段落。
- **修复建议**: 逐步补充，优先覆盖会 panic 的函数。

### D-031: lib.rs 模块列表与实际不一致

- **严重度**: Low
- **文件**: `src/lib.rs` L18-27
- **描述**: 文档注释列出 `challenge`、`human`、`fetch` 模块，实际为 `stealth`（含子模块）和 `http`。

---

## 十、其他关注点

### D-032: Browser::Drop 中 std::thread::spawn 清理临时目录

- **严重度**: Low
- **文件**: `src/browser/mod.rs` L125-138
- **描述**: Drop 中 spawn 新线程做 `remove_dir_all`。程序快速退出时清理线程可能未完成。`dir.contains("wisp-")` 检查不够严格。
- **修复建议**: 使用 `tempfile` crate 的自动清理机制。

### D-033: Engine::run_stream 的 unfold 驱动模式复杂脆弱

- **严重度**: Low
- **文件**: `src/crawl/runner.rs` L92-137
- **描述**: `stream::unfold` + `tokio::select!` + `biased` 驱动 driver future，逻辑复杂。driver panic 时 stream 静默结束。
- **修复建议**: 考虑 `tokio::spawn` + `mpsc` 通道简化。

### D-034: Scheduler 测试注释仍标 RED 但代码已修复

- **严重度**: Low
- **文件**: `src/crawl/scheduling/scheduler.rs` L196-206
- **描述**: 测试注释写「RED：当前 restore() 在 Fingerprint 模式下...」，但代码已修复（L153-158 直接 parse 回 u64）。注释误导后续开发者。
- **修复建议**: 删除 RED 标记，更新为描述性注释。

---

## 统计汇总

| 类别 | 数量 | 最高严重度 |
|------|------|-----------|
| 正确性缺陷 | 5 | Critical |
| 安全漏洞 | 3 | High |
| 架构设计 | 3 | Medium |
| 错误处理 | 3 | Low |
| 性能问题 | 3 | Medium |
| 代码质量 | 5 | Low |
| 测试覆盖 | 4 | Medium |
| 依赖配置 | 3 | Medium |
| 文档完整性 | 2 | Low |
| 其他关注 | 3 | Low |
| **合计** | **34** | — |

## 优先修复建议（Top 5）

1. **D-001** DomainBlocker O(n) 性能 — 影响浏览器模式每个子资源请求
2. **D-006** ProxyConfig 凭据泄露 — 安全合规问题
3. **D-003** rand_suffix 碰撞 — 并发场景下 Browser 启动失败
4. **D-005** std::sync::Mutex 违规 — 违反项目自身规范
5. **D-027** anyhow 残留 — 违反架构约定，增加编译依赖
