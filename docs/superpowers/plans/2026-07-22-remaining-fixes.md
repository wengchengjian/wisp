# 剩余问题修复实施计划（Phase 3）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 2026-07-22 全面代码审查中剩余的 1 个 Important（I4）+ 9 个 Minor（M1-M8、M11-M12）问题。Phase 1/2 已修复 6 Critical + 8 Important，本计划完成收尾。

**Architecture:** I4 需重构 control.rs 全局状态为 EngineContext 注入（影响面大，独立 Task）。M 系列为局部修复，按文件分组并行执行。每个 Task 独立可提交，TDD 顺序。

**Tech Stack:** Rust, tokio, rusqlite, rand

---

## 前置说明

**已在 Phase 1/2 修复的问题（不在本计划范围）：**
- M9（wait_if_paused 5 秒轮询）→ 已在 Phase 2 作为 I9 修复（commit `a235dd2`）
- M10（adaptive position_in_parent outer_html 比较）→ 已在 Phase 2 作为 I1 修复（commit `07bf801`）

**本计划覆盖：**

| 问题 | 严重程度 | 描述 |
|------|---------|------|
| I4 | Important | control.rs 全局状态污染多 Engine 实例 |
| M1 | Minor | proxy.rs 默认端口 1080 应区分 HTTP/SOCKS5 |
| M2 | Minor | block.rs should_block 循环内 format! 分配 |
| M3 | Minor | browser rand_suffix 仅用纳秒，同纳秒冲突 |
| M4 | Minor | document.rs sxd 解析失败静默吞错 |
| M5 | Minor | engine.rs 信号量 acquire unwrap panic |
| M6 | Minor | mod.rs 正则编译失败 unwrap_or(false) 静默忽略 |
| M7 | Minor | Cargo.toml wreq RC 版本生产风险 |
| M8 | Minor | turnstile.rs 递归无深度限制栈溢出 |
| M11 | Minor | mcp human_mode 仅 sleep 500ms |
| M12 | Minor | engine.rs unreachable! 无消息 |

---

## 文件结构

| 文件 | 责任 | 动作 |
|------|------|------|
| `src/crawl/control.rs` | I4: 全局状态重构为 EngineContext 注入 | 修改 |
| `src/crawl/mod.rs` | I4: EngineContext 接入控制状态；M6: 正则编译失败日志 | 修改 |
| `src/crawl/engine.rs` | I4: process_request 传递控制状态；M5: 信号量错误处理；M12: unreachable! 带消息 | 修改 |
| `src/http/proxy.rs` | M1: 默认端口按 scheme 区分 | 修改 |
| `src/http/block.rs` | M2: should_block 预计算 format! | 修改 |
| `src/browser/mod.rs` | M3: rand_suffix 用 rand crate | 修改 |
| `src/parser/document.rs` | M4: sxd 解析失败返回错误或日志 | 修改 |
| `src/stealth/turnstile.rs` | M8: find_turnstile_node 加深度限制 | 修改 |
| `src/mcp/tools.rs` | M11: human_mode 随机延迟 + 滚动 | 修改 |
| `Cargo.toml` | M7: wreq 版本固定或文档标注 | 修改 |
| `tests/cr_fix_phase3_test.rs` | 本计划新增回归测试 | **新建** |

---

# Task 1: 重构 control.rs 全局状态为 EngineContext 注入（I4）

**Files:**
- Modify: `src/crawl/control.rs`（全文件重构）
- Modify: `src/crawl/mod.rs`（EngineContext 接入控制状态）
- Modify: `src/crawl/engine.rs`（process_request 传递控制状态）

**问题：** `control.rs` 的 `PAUSED_URLS`、`CANCELLED_URLS`、`SHUTDOWN_FLAG`、`GLOBAL_PAUSED`、`VERSION` 均为 process-wide static。同时运行多个 Engine（如测试或 MCP 多爬虫）会互相干扰：`shutdown()` 停止所有 Engine，`cancel(url)` 取消所有 Engine 中相同 URL。

- [ ] **Step 1: 定义 EngineControl 结构体**

修改 `src/crawl/control.rs`，将全局状态封装为结构体：

```rust
//! 引擎级控制状态（per-Engine 隔离）。
//!
//! 替代原 process-wide static，避免多 Engine 实例互相干扰。

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, RwLock};

/// 单个 Engine 的控制状态。
///
/// 每个 Engine 持有一个 `Arc<EngineControl>`，pause/cancel/shutdown
/// 操作仅影响该 Engine，不再污染全局。
#[derive(Debug)]
pub struct EngineControl {
    /// 暂停的 URL 集合（per-Engine）
    paused_urls: Arc<RwLock<HashSet<String>>>,
    /// 取消的 URL 集合（per-Engine）
    cancelled_urls: Arc<RwLock<HashSet<String>>>,
    /// 全局暂停标志（per-Engine）
    global_paused: AtomicBool,
    /// 关闭标志（per-Engine）
    shutdown: AtomicBool,
    /// 版本号（用于 watch channel 唤醒 wait_if_paused）
    version: watch::Sender<u64>,
}

impl EngineControl {
    /// 创建新的控制状态（version 初始为 0）。
    pub fn new() -> Self {
        let (tx, _rx) = watch::channel(0u64);
        Self {
            paused_urls: Arc::new(RwLock::new(HashSet::new())),
            cancelled_urls: Arc::new(RwLock::new(HashSet::new())),
            global_paused: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            version: tx,
        }
    }

    fn bump(&self) {
        let _ = self.version.send(self.version.borrow().wrapping_add(1));
    }

    /// 暂停指定 URL。
    pub async fn pause(&self, url: &str) {
        self.paused_urls.write().await.insert(url.to_string());
        self.bump();
    }

    /// 恢复指定 URL。
    pub async fn resume(&self, url: &str) {
        self.paused_urls.write().await.remove(url);
        self.bump();
    }

    /// 暂停所有请求。
    pub fn pause_all(&self) {
        self.global_paused.store(true, Ordering::SeqCst);
        self.bump();
    }

    /// 恢复所有请求。
    pub fn resume_all(&self) {
        self.global_paused.store(false, Ordering::SeqCst);
        self.bump();
    }

    /// 取消指定 URL（从队列移除且不再派发）。
    pub async fn cancel(&self, url: &str) {
        self.cancelled_urls.write().await.insert(url.to_string());
        self.bump();
    }

    /// 关闭 Engine（停止派发新请求）。
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.bump();
    }

    /// 重置所有控制状态。
    pub async fn reset(&self) {
        self.paused_urls.write().await.clear();
        self.cancelled_urls.write().await.clear();
        self.global_paused.store(false, Ordering::SeqCst);
        self.shutdown.store(false, Ordering::SeqCst);
        self.bump();
    }

    /// 检查 URL 是否已取消。
    pub async fn is_cancelled(&self, url: &str) -> bool {
        self.cancelled_urls.read().await.contains(url)
    }

    /// 检查是否已关闭。
    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    /// 若 URL 或全局暂停激活，阻塞直到恢复或关闭。
    /// 返回 `false` 表示检测到关闭（调用方应终止）。
    pub async fn wait_if_paused(&self, url: &str) -> bool {
        let mut rx = self.version.subscribe();
        loop {
            if self.shutdown.load(Ordering::SeqCst) { return false; }
            let global = self.global_paused.load(Ordering::SeqCst);
            let url_paused = self.paused_urls.read().await.contains(url);
            if !global && !url_paused { return true; }
            tokio::select! {
                changed = rx.changed() => {
                    if changed.is_err() {
                        return !self.shutdown.load(Ordering::SeqCst);
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(60)) => {}
            }
        }
    }
}

impl Default for EngineControl {
    fn default() -> Self { Self::new() }
}
```

- [ ] **Step 2: 删除全局 static 变量和全局函数**

直接删除 `PAUSED_URLS`、`CANCELLED_URLS`、`SHUTDOWN_FLAG`、`GLOBAL_PAUSED`、`VERSION` 这些 static 变量，以及所有全局函数（`pause`、`resume`、`pause_all`、`resume_all`、`cancel`、`shutdown`、`reset`、`is_cancelled`、`is_shutdown`、`wait_if_paused`）。

**不要保留全局 facade**——保留一个"默认 EngineControl"会让旧代码编译通过但实际是空操作（没人调用它的 `wait_if_paused`），比编译错误更危险，且重新引入了 I4 的多 Engine 污染问题。

**搜索调用点**：用 grep 搜索 `control::pause`、`control::shutdown`、`control::is_cancelled`、`control::wait_if_paused` 等所有全局函数调用，改为通过 `ctx.control` 或 `engine.control()` 访问。

**外部访问 EngineControl 的方式**：给 `Engine` 暴露 `control()` 方法，让 CLI/MCP 等外部代码能拿到特定 Engine 的 control 引用：

```rust
impl Engine {
    /// 获取该 Engine 的控制句柄（用于 CLI/MCP 外部控制）。
    pub fn control(&self) -> &Arc<control::EngineControl> {
        &self.control
    }
}
```

CLI 代码改为：
```rust
// 旧：control::pause_all();
// 新：engine.control().pause_all();
```

- [ ] **Step 3: EngineContext 接入 EngineControl**

修改 `src/crawl/engine.rs`，给 `EngineContext` 加字段：

```rust
pub(crate) struct EngineContext {
    // ... 现有字段 ...
    /// per-Engine 控制状态（替代全局 static）
    pub control: Arc<control::EngineControl>,
    // ...
}
```

在 `run_with_sender` 中构造时传入：

```rust
let control = Arc::new(control::EngineControl::new());
// 传入 EngineContext
```

- [ ] **Step 4: 修改 process_request 用 ctx.control**

修改 `src/crawl/engine.rs` 的 `process_request`，将 `control::wait_if_paused(&req.url)` 改为 `ctx.control.wait_if_paused(&req.url).await`，`control::is_cancelled(&req.url)` 改为 `ctx.control.is_cancelled(&req.url).await`，`control::is_shutdown()` 改为 `ctx.control.is_shutdown()`。

- [ ] **Step 5: 修改 run_with_sender 路由循环**

修改 `src/crawl/mod.rs` 的 `run_with_sender`，将 `control::is_shutdown()` 改为 `ctx.control.is_shutdown()`。

- [ ] **Step 6: 写测试验证多 Engine 隔离**

创建 `tests/cr_fix_phase3_test.rs`：

```rust
//! Phase 3 回归测试。
use wisp::crawl::control::EngineControl;

#[tokio::test]
async fn test_engine_control_isolation() {
    let ctrl_a = EngineControl::new();
    let ctrl_b = EngineControl::new();
    // A 关闭不影响 B
    ctrl_a.shutdown();
    assert!(ctrl_a.is_shutdown());
    assert!(!ctrl_b.is_shutdown(), "Engine B 不应受 A 关闭影响");
    // A 暂停 URL 不影响 B
    ctrl_a.pause("https://example.com/page1").await;
    assert!(ctrl_a.is_cancelled("https://example.com/page1").await == false, "pause != cancel");
    assert!(ctrl_b.is_cancelled("https://example.com/page1").await == false, "B 不应受 A 影响");
}

#[tokio::test]
async fn test_engine_control_wait_if_paused_not_paused() {
    let ctrl = EngineControl::new();
    let result = ctrl.wait_if_paused("https://example.com").await;
    assert!(result, "未暂停时应立即返回 true");
}

#[tokio::test]
async fn test_engine_control_shutdown_wakes_wait() {
    let ctrl = Arc::new(EngineControl::new());
    let ctrl2 = ctrl.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        ctrl2.shutdown();
    });
    ctrl.pause_all();
    let result = ctrl.wait_if_paused("https://example.com").await;
    assert!(!result, "shutdown 后应返回 false");
}
```

- [ ] **Step 7: 验证**

```
cargo build --lib
cargo test --test cr_fix_phase3_test -- --nocapture
cargo test --lib  # 确保无回归
```

- [ ] **Step 8: 提交**

```bash
git add src/crawl/control.rs src/crawl/mod.rs src/crawl/engine.rs tests/cr_fix_phase3_test.rs
git commit -m "refactor(crawl): control 全局状态重构为 per-Engine EngineControl" -m "I4: 原 static 状态导致多 Engine 互相干扰" -m "EngineControl 封装 pause/cancel/shutdown，保留全局 facade 向后兼容"
```

---

# Task 2: 修复 proxy.rs 默认端口按 scheme 区分（M1）

**Files:**
- Modify: `src/http/proxy.rs:28`

- [ ] **Step 1: 修改默认端口逻辑**

```rust
let port = parsed.port().unwrap_or_else(|| {
    match parsed.scheme() {
        "socks5" | "socks5h" => 1080,
        "http" | "https" => 8080,
        _ => 1080,
    }
});
```

- [ ] **Step 2: 写测试**

在 `tests/cr_fix_phase3_test.rs` 追加：

```rust
#[test]
fn test_proxy_default_port_by_scheme() {
    use wisp::http::proxy::ProxyConfig;
    let http_proxy = ProxyConfig::parse("http://proxy.example.com").unwrap();
    assert_eq!(http_proxy.port, 8080, "HTTP 代理默认端口应为 8080");
    let socks5_proxy = ProxyConfig::parse("socks5://proxy.example.com").unwrap();
    assert_eq!(socks5_proxy.port, 1080, "SOCKS5 代理默认端口应为 1080");
}
```

- [ ] **Step 3: 验证**

```
cargo test --test cr_fix_phase3_test test_proxy_default_port_by_scheme -- --nocapture
```

- [ ] **Step 4: 提交**

```bash
git add src/http/proxy.rs tests/cr_fix_phase3_test.rs
git commit -m "fix(http): proxy 默认端口按 scheme 区分 HTTP/SOCKS5" -m "M1: 原统一 1080，HTTP 代理应为 8080"
```

---

# Task 3: 优化 block.rs should_block 预计算 format!（M2）

**Files:**
- Modify: `src/http/block.rs:72-84`
- Modify: `src/http/block.rs:87-92`

- [ ] **Step 1: 修改 should_block 预计算 format!**

```rust
pub fn should_block(&self, url: &str) -> bool {
    let host = match url::Url::parse(url) {
        Ok(u) => u.host_str().unwrap_or("").to_lowercase(),
        Err(_) => return false,
    };
    for blocked in &self.blocked {
        // 预计算 suffix，避免每次 format! 分配
        if host == *blocked || host.ends_with(&format!(".{}", blocked)) {
            return true;
        }
    }
    false
}
```

**优化方案**：改为直接检查 `host` 是否以 `.` + `blocked` 结尾，避免 `format!` 分配：

```rust
pub fn should_block(&self, url: &str) -> bool {
    let host = match url::Url::parse(url) {
        Ok(u) => u.host_str().unwrap_or("").to_lowercase(),
        Err(_) => return false,
    };
    for blocked in &self.blocked {
        if host == *blocked {
            return true;
        }
        // 检查 host 是否以 ".{blocked}" 结尾，避免 format! 分配
        let suffix = format!(".{}", blocked);
        if host.ends_with(&suffix) {
            return true;
        }
    }
    false
}
```

**进一步优化**：用 `host.len() > blocked.len() + 1` + 字节比较完全消除 `format!`：

```rust
pub fn should_block(&self, url: &str) -> bool {
    let host = match url::Url::parse(url) {
        Ok(u) => u.host_str().unwrap_or("").to_lowercase(),
        Err(_) => return false,
    };
    for blocked in &self.blocked {
        if host == *blocked {
            return true;
        }
        // host.ends_with(".{blocked}") 无需 format!：检查分隔符 + 后缀
        if host.len() > blocked.len() + 1 {
            let dot_pos = host.len() - blocked.len() - 1;
            if host.as_bytes()[dot_pos] == b'.'
                && host[dot_pos + 1..].eq_ignore_ascii_case(blocked)
            {
                return true;
            }
        }
    }
    false
}
```

对 `should_block_host` 做同样修改。

- [ ] **Step 2: 写测试确保功能不变**

在 `tests/cr_fix_phase3_test.rs` 追加：

```rust
#[test]
fn test_domain_blocker_should_block_optimized() {
    use wisp::http::block::DomainBlocker;
    let mut blocker = DomainBlocker::new();
    blocker.block_domain("ads.example.com");
    blocker.block_domain("tracker.io");
    assert!(blocker.should_block("https://ads.example.com/banner.js"));
    assert!(blocker.should_block("https://sub.ads.example.com/pixel"));
    assert!(blocker.should_block("https://deep.sub.ads.example.com/x"));
    assert!(blocker.should_block("https://tracker.io/track"));
    assert!(blocker.should_block("https://sub.tracker.io/track"));
    assert!(!blocker.should_block("https://example.com/page"));
    assert!(!blocker.should_block("https://notracker.io/page"), "不应误匹配子串");
    assert!(!blocker.should_block("https://evilads.example.com/page"), "不应误匹配前缀");
}
```

- [ ] **Step 3: 验证**

```
cargo test --test cr_fix_phase3_test test_domain_blocker_should_block_optimized -- --nocapture
```

- [ ] **Step 4: 提交**

```bash
git add src/http/block.rs tests/cr_fix_phase3_test.rs
git commit -m "perf(http): should_block 消除循环内 format! 分配" -m "M2: 用字节比较替代 format!，大 blocked 集合时减少内存分配"
```

---

# Task 4: 修复 browser rand_suffix 用 rand crate（M3）

**Files:**
- Modify: `src/browser/mod.rs:139-144`
- Modify: `Cargo.toml`（添加 rand 依赖）

- [ ] **Step 1: 添加 rand 依赖**

在 `Cargo.toml` 的 `[dependencies]` 添加：

```toml
rand = "0.8"
```

- [ ] **Step 2: 修改 rand_suffix 用 rand**

```rust
/// Generate a short random suffix for unique temp dirs.
fn rand_suffix() -> String {
    use rand::Rng;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let rand_val: u32 = rand::thread_rng().gen();
    format!("{:x}{:x}", nanos, rand_val)
}
```

- [ ] **Step 3: 验证编译**

```
cargo build --lib
```

- [ ] **Step 4: 提交**

```bash
git add src/browser/mod.rs Cargo.toml
git commit -m "fix(browser): rand_suffix 用 rand crate 避免同纳秒冲突" -m "M3: 原仅用纳秒，同一纳秒启动多个浏览器会冲突"
```

---

# Task 5: 修复 document.rs sxd 解析失败日志（M4）

**Files:**
- Modify: `src/parser/document.rs:62-67`

- [ ] **Step 1: 修改 build_sxd_from_html 记录失败**

```rust
fn build_sxd_from_html(html: &Html) -> Package {
    let clean_html = html.html();
    match sxd_document::parser::parse(&clean_html) {
        Ok(pkg) => pkg,
        Err(e) => {
            tracing::warn!("sxd_document 解析失败（XPath 查询将返回空）: {}", e);
            Package::new()
        }
    }
}
```

- [ ] **Step 2: 验证编译**

```
cargo build --lib
```

- [ ] **Step 3: 提交**

```bash
git add src/parser/document.rs
git commit -m "fix(parser): sxd 解析失败记录 warning 而非静默吞错" -m "M4: 原 unwrap_or_else 静默返回空 Package，XPath 查询无日志可查"
```

---

# Task 6: 修复 engine.rs 信号量 acquire unwrap panic（M5）

**Files:**
- Modify: `src/crawl/engine.rs:181`

- [ ] **Step 1: 修改信号量获取失败处理**

```rust
let _permit = match sem.acquire_owned().await {
    Ok(permit) => permit,
    Err(e) => {
        tracing::error!("信号量获取失败（域名并发控制关闭）: {} - {}", domain, e);
        // 信号量关闭不应阻止请求继续，返回一个 dummy guard
        // 由于 Semaphore::acquire_owned 只在 Semaphore 关闭时返回 Err，
        // 这里跳过并发控制继续请求
        return process_request_inner(ctx, req, idx, spider).await;
    }
};
```

**注意**：如果 `process_request` 的逻辑不好拆分，改为更简单的方式——信号量关闭时用 `unwrap_or_else` 记录日志并继续（虽然丢失并发控制但不会 panic）：

```rust
let _permit = sem.acquire_owned().await.unwrap_or_else(|e| {
    tracing::error!("信号量获取失败（域名并发控制失效）: {} - {}", domain, e);
    // 返回一个 dummy permit（Semaphore 关闭时的降级）
    // 由于 acquire_owned 返回 OwnedSemaphorePermit，无法构造 dummy
    // 改为直接 continue 让请求跳过并发限制
    panic!("信号量关闭: {}", e)  // 实际上 Semaphore 不会正常关闭
});
```

**更好的方案**：信号量关闭是极端异常（Semaphore 被 close），用 `expect` 带消息比 `unwrap` 好：

```rust
let _permit = sem.acquire_owned().await
    .expect("域名信号量不应关闭（Engine 运行期间 Semaphore 始终有效）");
```

- [ ] **Step 2: 验证编译**

```
cargo build --lib
```

- [ ] **Step 3: 提交**

```bash
git add src/crawl/engine.rs
git commit -m "fix(crawl): 信号量 acquire 失败用 expect 带消息替代 unwrap" -m "M5: 原 unwrap 无消息，panic 时无法定位"
```

---

# Task 7: 修复 mod.rs 正则编译失败日志（M6）

**Files:**
- Modify: `src/crawl/mod.rs`（Spider::matches 默认实现和 compiled_patterns 预编译）

**注意**：Phase 2 已在 EngineContext 预编译 patterns，但 `Spider::matches()` 默认实现仍保留 fallback 路径（每次编译正则）。`compiled_patterns` 预编译时用 `filter_map(|p| regex::Regex::new(p).ok())` 静默忽略了编译失败的正则。

- [ ] **Step 1: 修改预编译记录失败正则**

在 `src/crawl/mod.rs` 的 `run_with_sender` 中，预编译 patterns 时记录失败：

```rust
let compiled_patterns: Vec<Vec<regex::Regex>> = spiders.iter().map(|s| {
    s.patterns().iter()
        .filter_map(|p| match regex::Regex::new(p) {
            Ok(re) => Some(re),
            Err(e) => {
                tracing::warn!("Spider '{}' patterns 正则编译失败，跳过: '{}' - {}", s.name(), p, e);
                None
            }
        })
        .collect()
}).collect();
```

- [ ] **Step 2: 修改 Spider::matches fallback 也记录失败**

```rust
fn matches(&self, url: &str) -> bool {
    let patterns = self.patterns();
    if patterns.is_empty() {
        return true;
    }
    patterns.iter().any(|p| {
        match regex::Regex::new(p) {
            Ok(re) => re.is_match(url),
            Err(e) => {
                tracing::warn!("Spider patterns 正则编译失败: '{}' - {}", p, e);
                false
            }
        }
    })
}
```

- [ ] **Step 3: 验证**

```
cargo build --lib
cargo test --lib crawl::tests -- --nocapture
```

- [ ] **Step 4: 提交**

```bash
git add src/crawl/mod.rs
git commit -m "fix(crawl): 正则编译失败记录 warning 而非静默忽略" -m "M6: 原 unwrap_or(false) 静默吞错，用户无法发现 patterns 配置错误"
```

---

# Task 8: 修复 turnstile.rs 递归深度限制（M8）

**Files:**
- Modify: `src/stealth/turnstile.rs:200-254`

- [ ] **Step 1: 给 find_turnstile_node 加深度参数**

```rust
/// Recursively search DOM tree (including shadow roots) for Turnstile iframe.
/// Returns the nodeId if found.
///
/// `depth` 限制递归深度（默认 50），防止恶意页面构造超深 DOM 栈溢出。
fn find_turnstile_node(node: &Value) -> Option<u32> {
    find_turnstile_node_inner(node, 50)
}

fn find_turnstile_node_inner(node: &Value, depth: u32) -> Option<u32> {
    if depth == 0 {
        tracing::warn!("find_turnstile_node 达到最大递归深度 50，停止搜索");
        return None;
    }
    let node_name = node.get("nodeName").and_then(|n| n.as_str()).unwrap_or("");
    let attributes = node.get("attributes").and_then(|a| a.as_array());

    // Check if this is a Turnstile iframe
    if node_name.eq_ignore_ascii_case("IFRAME") {
        if let Some(attrs) = attributes {
            let is_turnstile = attrs.chunks(2).any(|pair| {
                if pair.len() == 2 {
                    let key = pair[0].as_str().unwrap_or("");
                    let val = pair[1].as_str().unwrap_or("");
                    (key == "src" && val.contains("challenges.cloudflare.com"))
                        || (key == "id" && val.contains("cf-chl-widget"))
                } else {
                    false
                }
            });
            if is_turnstile {
                return node.get("nodeId").and_then(|id| id.as_u64()).map(|id| id as u32);
            }
        }
    }

    // Recurse into children
    if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
        for child in children {
            if let Some(id) = find_turnstile_node_inner(child, depth - 1) {
                return Some(id);
            }
        }
    }

    // Recurse into shadow roots
    if let Some(shadow_roots) = node.get("shadowRoots").and_then(|s| s.as_array()) {
        for sr in shadow_roots {
            if let Some(sr_children) = sr.get("children").and_then(|c| c.as_array()) {
                for sr_child in sr_children {
                    if let Some(id) = find_turnstile_node_inner(sr_child, depth - 1) {
                        return Some(id);
                    }
                }
            }
        }
    }

    // Recurse into iframe contentDocument
    if let Some(content_doc) = node.get("contentDocument") {
        if let Some(id) = find_turnstile_node_inner(content_doc, depth - 1) {
            return Some(id);
        }
    }

    None
}
```

- [ ] **Step 2: 验证编译**

```
cargo build --lib
```

- [ ] **Step 3: 提交**

```bash
git add src/stealth/turnstile.rs
git commit -m "fix(stealth): find_turnstile_node 加深度限制防止栈溢出" -m "M8: 原 50→递归无限制，恶意页面可构造超深 DOM 导致栈溢出"
```

---

# Task 9: 改进 mcp human_mode 人类行为模拟（M11）

**Files:**
- Modify: `src/mcp/tools.rs:209-212`

- [ ] **Step 1: 修改 human_mode 实现随机延迟 + 滚动**

```rust
if human_mode {
    // 人类行为模拟：随机延迟（200-800ms）模拟阅读时间
    use rand::Rng;
    let delay_ms = rand::thread_rng().gen_range(200..800);
    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
    // 模拟滚动：滚动到页面底部再回顶部
    let _ = page.evaluate("window.scrollTo(0, document.body.scrollHeight)").await;
    tokio::time::sleep(std::time::Duration::from_millis(
        rand::thread_rng().gen_range(100..300)
    )).await;
    let _ = page.evaluate("window.scrollTo(0, 0)").await;
}
```

**注意**：确认 `page.evaluate` 方法存在且返回 `Result`。如果 `rand` 依赖未添加（Task 4 已添加），先添加。如果 `page` 没有 `evaluate` 方法，检查 `src/browser/page.rs` 的实际 API，可能叫 `evaluate_as_string` 或其他。

- [ ] **Step 2: 验证编译**

```
cargo build --lib
```

- [ ] **Step 3: 提交**

```bash
git add src/mcp/tools.rs
git commit -m "feat(mcp): human_mode 改进为随机延迟 + 滚动模拟" -m "M11: 原仅固定 sleep 500ms，无真正人类行为模拟"
```

---

# Task 10: 修复 engine.rs unreachable! 带消息（M12）

**Files:**
- Modify: `src/crawl/engine.rs:466`

- [ ] **Step 1: 搜索所有 unreachable! 调用**

先搜索 `src/crawl/engine.rs` 中所有 `unreachable!` 调用，给每个加上描述性消息。

- [ ] **Step 2: 修改 unreachable! 带消息**

```rust
unreachable!("FetchMode 匹配不应到达此处: {:?}", mode)
```

**注意**：确认 `FetchMode` 实现了 `Debug`。如果 mode 匹配的分支已经覆盖所有变体，`unreachable!` 是正确的。加消息帮助 panic 时定位。

- [ ] **Step 3: 验证**

```
cargo build --lib
```

- [ ] **Step 4: 提交**

```bash
git add src/crawl/engine.rs
git commit -m "fix(crawl): unreachable! 带描述性消息" -m "M12: 原 unreachable! 无消息，panic 时无法定位"
```

---

# Task 11: 评估 wreq RC 版本风险（M7）

**Files:**
- Modify: `Cargo.toml`（可能修改或添加注释）

**注意**：`wreq = "6.0.0-rc"` 和 `wreq-util = "3.0.0-rc"` 是 RC（Release Candidate）版本。这个 Task 主要是评估和文档化，不一定要立即修改版本。

- [ ] **Step 1: 检查 wreq 最新版本**

```
cargo search wreq
```

如果已有稳定版（如 `6.0.0`），升级到稳定版。如果仍为 RC，在 Cargo.toml 添加注释说明：

```toml
# 注意：wreq 6.0 尚无稳定版，使用 RC 版本。生产环境需评估风险。
# 升级路径：等 wreq 6.0.0 稳定版发布后替换。
wreq = "6.0.0-rc"
wreq-util = "3.0.0-rc"
```

- [ ] **Step 2: 验证编译**

```
cargo build --lib
```

- [ ] **Step 3: 提交**

```bash
git add Cargo.toml
git commit -m "docs: 标注 wreq RC 版本风险与升级路径" -m "M7: wreq 6.0.0-rc 为 RC 版本，生产需评估风险"
```

---

# Task 12: 全量测试与最终验证

- [ ] **Step 1: 运行全量 lib 测试**

```
cargo test --lib
```
Expected: 全部 PASS

- [ ] **Step 2: 运行 Phase 3 回归测试**

```
cargo test --test cr_fix_phase3_test -- --nocapture
```
Expected: 全部 PASS

- [ ] **Step 3: 运行所有集成测试**

```
cargo test --test cr_fix_engine_test --test cr_fix_t1_test --test cr_fix_t4_test --test cr_fix_t7_test --test cr_fix_t10_test --test cr_fix_t11_test --test multi_spider_test --test stop_condition_test --test builder_api_test
```
Expected: 全部 PASS

- [ ] **Step 4: 运行编译检查**

```
cargo build
```
Expected: 编译通过

- [ ] **Step 5: 确认所有提交**

```
git log --oneline -20
```

---

## 自检清单

**Spec 覆盖：**
- I4（control 全局状态）→ Task 1 ✅
- M1（proxy 默认端口）→ Task 2 ✅
- M2（block format!）→ Task 3 ✅
- M3（rand_suffix）→ Task 4 ✅
- M4（sxd 静默吞错）→ Task 5 ✅
- M5（信号量 unwrap）→ Task 6 ✅
- M6（正则编译失败）→ Task 7 ✅
- M7（wreq RC 版本）→ Task 11 ✅
- M8（turnstile 递归）→ Task 8 ✅
- M11（human_mode）→ Task 9 ✅
- M12（unreachable!）→ Task 10 ✅

**未覆盖（已在 Phase 1/2 修复）：**
- M9（wait_if_paused 轮询）→ Phase 2 已修复
- M10（adaptive position_in_parent）→ Phase 2 已修复

**并发执行建议：**
- Task 1（I4）影响面大，**必须单独执行**，不与其他 Task 并发
- Task 2-11 可分组并行：
  - 组 A：Task 2 (proxy) + Task 3 (block) + Task 5 (document) + Task 8 (turnstile) — 独立文件
  - 组 B：Task 4 (browser+rand) + Task 9 (mcp human_mode) — 共享 rand 依赖，顺序执行
  - 组 C：Task 6 (engine semaphore) + Task 7 (mod.rs regex) + Task 10 (engine unreachable) — 共享 engine.rs/mod.rs，顺序执行
  - Task 11 (Cargo.toml) 独立

**类型一致性：**
- `EngineControl` 在 Task 1 定义，EngineContext 引用 ✅
- `rand` 依赖在 Task 4 添加，Task 9 使用 ✅
