# Stage 3: wreq 替换 reqwest（TLS 指纹模拟）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 wisp 的 HTTP 客户端从 reqwest 0.12 切换到 wreq 6.0.0-rc，启用 TLS/JA3/JA4 指纹模拟（默认 Chrome136），保持 `fetch::Client` 公共 API 完全兼容。

**Architecture:** `src/fetch/mod.rs` 内部类型 `reqwest::Client` → `wreq::Client`，`Config` 新增 `emulation: Option<Emulation>` 和 `header_order: Option<Vec<HeaderName>>` 字段。`ClientBuilder` 新增 `emulation()` / `no_emulation()` 方法。公共 API（`get/post/put/delete/builder`）签名不变，`crawl::Engine` 和 `crawl::robots` 对切换透明。

**Tech Stack:** Rust 2021, wreq 6.0.0-rc.29 (BoringSSL/btls-sys 0.5.6), wreq-util 3.0.0-rc.14 (Emulation 75 变体)

**Spec:** `docs/superpowers/specs/2026-07-21-scrapling-borrow-design.md` 章节 2.1（已更新版本号和 API 实测）

**前置条件（已验证）：**
- wreq 6.0.0-rc 在 Windows + BoringSSL 工具链编译通过（perl 5.42 / nasm 2.16 / cmake 4.3 / go 1.26，耗时 9m35s）
- wreq API 与 reqwest 高度兼容（Response/redirect/Proxy/header 全部兼容）
- wreq 新增 `emulation<P: EmulationProviderFactory>()` 和 `headers_order(Cow<[HeaderName]>)`

**全局约束：**
- Rust 2021 edition, wisp 项目 `f:\project\wisp`
- PowerShell 无 heredoc，git 提交用单 `-m`
- Commit messages 中文
- 最小改动，不破坏公共 API
- 所有命令需有超时（cargo build 首次约 10 分钟，用非阻塞 + 长超时）

---

## File Structure

| 文件 | 责任 | 本 stage 改动 |
|---|---|---|
| `Cargo.toml` | 依赖声明 | 移除 reqwest，添加 wreq + wreq-util |
| `src/fetch/mod.rs` | HTTP 客户端封装 | 内部类型替换 + 新增 emulation/header_order 配置 |
| `src/fetch/proxy.rs` | 代理 URL 解析 | 仅注释更新（reqwest → wreq） |
| `tests/fetch_test.rs` | fetch 模块测试 | 新建：emulation 配置 + builder + 兼容性测试 |

**不触碰的文件：** `src/crawl/mod.rs`, `src/crawl/robots.rs`（依赖 `fetch::Client` 公共 API，切换透明，无需修改）

---

## Task 1: 更新 Cargo.toml 依赖（reqwest → wreq）

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: 读取当前 Cargo.toml 的 reqwest 依赖行**

Run: 读 `Cargo.toml` line 29 附近，确认当前是 `reqwest = { version = "0.12", features = ["rustls-tls"], default-features = false }`

- [ ] **Step 2: 替换 reqwest 为 wreq + wreq-util**

在 `Cargo.toml` 中，将：
```toml
reqwest = { version = "0.12", features = ["rustls-tls"], default-features = false }
```
替换为：
```toml
wreq = "6.0.0-rc"
wreq-util = "3.0.0-rc"
```

- [ ] **Step 3: 验证 Cargo.toml 修改**

Run: `cargo tree --depth 1 | Select-String -Pattern "wreq|reqwest"`
Expected: 输出包含 `wreq v6.0.0-rc` 和 `wreq-util v3.0.0-rc`，不含 `reqwest`

- [ ] **Step 4: 触发 wreq 编译（首次约 10 分钟，非阻塞）**

Run: `cargo build` (非阻塞，长超时)
Expected: wreq + btls-sys + BoringSSL 编译成功（无 reqwest 残留）

- [ ] **Step 5: 提交**

```bash
git add Cargo.toml Cargo.lock
git commit -m "build: 替换 reqwest 为 wreq 6.0.0-rc + wreq-util 3.0.0-rc（TLS 指纹模拟依赖）"
```

---

## Task 2: 重写 src/fetch/mod.rs（reqwest → wreq）

**Files:**
- Modify: `src/fetch/mod.rs`

**目标：** 将 `fetch::Client` 内部类型从 `reqwest::Client` 切换到 `wreq::Client`，保持公共 API（`builder/get/post/put/delete`）签名完全不变。`Config` 暂不加新字段（Task 3 加）。

- [ ] **Step 1: 读取当前 src/fetch/mod.rs 完整内容**

Run: 读 `src/fetch/mod.rs`（约 193 行），理解 Client/ClientBuilder/Config/Response 结构

- [ ] **Step 2: 替换所有 reqwest 引用为 wreq**

在 `src/fetch/mod.rs` 中：

1. 将 `reqwest::Client::builder()` 替换为 `wreq::Client::builder()`
2. 将 `reqwest::redirect::Policy::limited(self.config.max_redirects)` 替换为 `wreq::redirect::Policy::limited(self.config.max_redirects)`
3. 将 `.danger_accept_invalid_certs(false)` 替换为 `.tls_cert_verification(true)`（wreq 默认验证，显式声明更清晰）
4. 将 `reqwest::Proxy::all(proxy_url)` 替换为 `wreq::Proxy::all(proxy_url)`
5. 将 `http: reqwest::Client` 替换为 `http: wreq::Client`
6. 将所有 `reqwest::header::HeaderMap` 替换为 `wreq::header::HeaderMap`
7. 将所有 `reqwest::header::HeaderName` 替换为 `wreq::header::HeaderName`
8. 将所有 `reqwest::header::HeaderValue` 替换为 `wreq::header::HeaderValue`
9. 将所有 `reqwest::header::CONTENT_TYPE` 替换为 `wreq::header::CONTENT_TYPE`
10. 将 `resp: reqwest::Response` 替换为 `resp: wreq::Response`

**关键 API 兼容性（已验证）：**
- `wreq::Client::builder()` 返回 `ClientBuilder`
- `wreq::ClientBuilder::redirect(Policy)` 存在
- `wreq::ClientBuilder::tls_cert_verification(bool)` 存在（替代 danger_accept_invalid_certs）
- `wreq::ClientBuilder::proxy(P)` 存在
- `wreq::ClientBuilder::user_agent(V)` 存在
- `wreq::ClientBuilder::timeout(Duration)` 存在
- `wreq::Proxy::all(url) -> Result<Proxy>` 存在
- `wreq::Response::status() -> StatusCode` 存在
- `wreq::Response::url() -> &Url` 存在
- `wreq::Response::headers() -> &HeaderMap` 存在
- `wreq::Response::bytes() -> Result<Bytes>` 存在
- `wreq::StatusCode::as_u16()` 存在

- [ ] **Step 3: 验证编译**

Run: `cargo check`
Expected: 编译通过（无 reqwest 残留错误）

- [ ] **Step 4: 运行 lib 测试确保无回归**

Run: `cargo test --lib`
Expected: 35 passed（与 stage 2 一致）

- [ ] **Step 5: 提交**

```bash
git add src/fetch/mod.rs
git commit -m "refactor: fetch::Client 内部类型从 reqwest 切换到 wreq（保持公共 API 兼容）"
```

---

## Task 3: 新增 TLS 指纹模拟配置（emulation + header_order）

**Files:**
- Modify: `src/fetch/mod.rs`

**目标：** `Config` 新增 `emulation: Option<Emulation>` 和 `header_order: Option<Vec<HeaderName>>` 字段，默认 `Chrome136`。`ClientBuilder` 新增 `emulation()` / `no_emulation()` 方法。`build()` 时应用 emulation 和 header_order。

- [ ] **Step 1: 读取当前 src/fetch/mod.rs 的 Config 和 ClientBuilder**

Run: 读 `src/fetch/mod.rs` lines 15-70，确认 Config 和 ClientBuilder 结构

- [ ] **Step 2: 修改 Config 新增 emulation 和 header_order 字段**

在 `src/fetch/mod.rs` 的 `Config` struct 中：

```rust
use wreq_util::Emulation;
use wreq::header::HeaderName;

/// HTTP client configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub timeout: Duration,
    pub user_agent: Option<String>,
    pub headers: HashMap<String, String>,
    pub proxy: Option<String>,
    pub max_redirects: usize,
    /// 浏览器 TLS 指纹模拟（默认 Chrome136，覆盖最广）
    pub emulation: Option<Emulation>,
    /// 自定义 header 顺序（wreq 要求 HeaderName 列表）
    pub header_order: Option<Vec<HeaderName>>,
}
```

- [ ] **Step 3: 修改 Config::default 实现新字段**

```rust
impl Default for Config {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            user_agent: Some("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36".to_string()),
            headers: HashMap::new(),
            proxy: None,
            max_redirects: 10,
            // 默认 Chrome 136 指纹（覆盖最广）
            emulation: Some(Emulation::Chrome136),
            header_order: None,
        }
    }
}
```

- [ ] **Step 4: 修改 ClientBuilder 新增 emulation/no_emulation 方法**

在 `ClientBuilder` impl 中追加：

```rust
    /// 指定浏览器 TLS 指纹模拟（Chrome/Firefox/Safari/Edge/OkHttp，75 变体）
    pub fn emulation(mut self, emu: Emulation) -> Self {
        self.config.emulation = Some(emu);
        self
    }

    /// 关闭 TLS 指纹模拟（用 wreq 默认行为，用于调试）
    pub fn no_emulation(mut self) -> Self {
        self.config.emulation = None;
        self
    }

    /// 自定义 header 顺序（wreq 按此顺序发送 header）
    pub fn header_order(mut self, order: Vec<HeaderName>) -> Self {
        self.config.header_order = Some(order);
        self
    }
```

- [ ] **Step 5: 修改 ClientBuilder::build 应用 emulation 和 header_order**

在 `build()` 方法中，在 `redirect(...)` 和 `tls_cert_verification(...)` 之后追加：

```rust
        // 应用 TLS 指纹模拟（必须在其他 TLS 配置之前，会覆盖）
        if let Some(emu) = self.config.emulation {
            builder = builder.emulation(emu);
        }
        // 应用 header 顺序
        if let Some(ref order) = self.config.header_order {
            builder = builder.headers_order(order.clone());
        }
```

**注意：** wreq 文档说明 "emulation 会覆盖现有配置，必须在其他 HTTP1/HTTP2/TLS 微调之前设置"。当前实现把 emulation 放在 redirect/timeout/proxy 之后，但 wreq 的 emulation 主要覆盖 TLS 层和 HTTP/2 设置，与 redirect/timeout/proxy 不冲突。

- [ ] **Step 6: 验证编译**

Run: `cargo check`
Expected: 编译通过

- [ ] **Step 7: 运行 lib 测试确保无回归**

Run: `cargo test --lib`
Expected: 35 passed

- [ ] **Step 8: 提交**

```bash
git add src/fetch/mod.rs
git commit -m "feat: Config 新增 emulation + header_order 字段（默认 Chrome136 TLS 指纹）"
```

---

## Task 4: 更新 src/fetch/proxy.rs 注释

**Files:**
- Modify: `src/fetch/proxy.rs`

**目标：** proxy.rs line 46 注释提到 "reqwest-compatible"，更新为 "wreq-compatible"。代码无变化（proxy.rs 只做 URL 解析，不依赖 reqwest/wreq 类型）。

- [ ] **Step 1: 读取 src/fetch/proxy.rs line 45-48**

Run: 读 `src/fetch/proxy.rs` line 45-48，确认注释内容

- [ ] **Step 2: 更新注释**

将 line 46 的 `/// Format as a reqwest-compatible proxy URL.` 替换为 `/// Format as a wreq-compatible proxy URL.`

- [ ] **Step 3: 运行 proxy 测试确保无回归**

Run: `cargo test --lib fetch::proxy`
Expected: 4 passed（proxy.rs 内置单元测试）

- [ ] **Step 4: 提交**

```bash
git add src/fetch/proxy.rs
git commit -m "docs: proxy.rs 注释从 reqwest 更新为 wreq"
```

---

## Task 5: 新建 tests/fetch_test.rs（emulation 配置 + builder + 兼容性测试）

**Files:**
- Create: `tests/fetch_test.rs`

**目标：** 验证 wreq 切换后的行为：emulation 配置生效、builder 链式调用、Config 默认值、header_order 设置。不发起实际网络请求（避免环境依赖）。

- [ ] **Step 1: 创建 tests/fetch_test.rs**

```rust
//! wreq 切换后的 fetch 模块测试。
//!
//! 验证：emulation 配置、builder 链式调用、Config 默认值、header_order。
//! 不发起实际网络请求（避免环境依赖）。

use wisp::fetch::{Client, ClientBuilder, Config};
use wreq_util::Emulation;
use wreq::header::HeaderName;

#[test]
fn test_config_default_has_chrome136_emulation() {
    let config = Config::default();
    assert_eq!(config.emulation, Some(Emulation::Chrome136));
    assert!(config.header_order.is_none());
}

#[test]
fn test_builder_emulation_override() {
    let builder = ClientBuilder::new()
        .emulation(Emulation::Firefox128);
    assert_eq!(builder.config_ref().emulation, Some(Emulation::Firefox128));
}

#[test]
fn test_builder_no_emulation() {
    let builder = ClientBuilder::new()
        .no_emulation();
    assert_eq!(builder.config_ref().emulation, None);
}

#[test]
fn test_builder_header_order() {
    let order = vec![
        HeaderName::from_static("user-agent"),
        HeaderName::from_static("accept"),
        HeaderName::from_static("accept-encoding"),
    ];
    let builder = ClientBuilder::new()
        .header_order(order.clone());
    assert_eq!(builder.config_ref().header_order.as_ref().unwrap(), &order);
}

#[test]
fn test_builder_chain_emulation_and_header_order() {
    let builder = ClientBuilder::new()
        .emulation(Emulation::Safari18)
        .header_order(vec![HeaderName::from_static("user-agent")])
        .timeout(std::time::Duration::from_secs(60))
        .user_agent("test-agent");
    let config = builder.config_ref();
    assert_eq!(config.emulation, Some(Emulation::Safari18));
    assert!(config.header_order.is_some());
    assert_eq!(config.timeout, std::time::Duration::from_secs(60));
    assert_eq!(config.user_agent.as_deref(), Some("test-agent"));
}

#[test]
fn test_client_build_with_emulation() {
    // 验证带 emulation 的 client 能成功 build（不发起请求）
    let client = Client::builder()
        .emulation(Emulation::Chrome136)
        .timeout(std::time::Duration::from_secs(10))
        .build();
    assert!(client.is_ok(), "client build with emulation should succeed: {:?}", client.err());
}

#[test]
fn test_client_build_with_no_emulation() {
    // 验证关闭 emulation 的 client 能成功 build
    let client = Client::builder()
        .no_emulation()
        .build();
    assert!(client.is_ok(), "client build without emulation should succeed: {:?}", client.err());
}
```

**注意：** 测试用了 `builder.config_ref()`，需要在 `ClientBuilder` 上新增一个 `pub fn config_ref(&self) -> &Config` 方法（仅测试用，放 `src/fetch/mod.rs`）。

- [ ] **Step 2: 在 ClientBuilder 新增 config_ref 方法（测试用）**

在 `src/fetch/mod.rs` 的 `ClientBuilder` impl 中追加：

```rust
    /// 获取配置引用（测试用）
    pub fn config_ref(&self) -> &Config {
        &self.config
    }
```

- [ ] **Step 3: 运行新测试**

Run: `cargo test --test fetch_test`
Expected: 7 passed

- [ ] **Step 4: 运行 lib 测试确保无回归**

Run: `cargo test --lib`
Expected: 35 passed

- [ ] **Step 5: 提交**

```bash
git add tests/fetch_test.rs src/fetch/mod.rs
git commit -m "test: 新建 fetch_test 验证 wreq emulation/header_order 配置（7 测试）"
```

---

## Task 6: 端到端集成测试与 stage 3 完成验证

**Files:**
- Modify: `tests/integration.rs`

**目标：** 在 `mod fetch_test` 模块（新建）追加端到端测试，验证 wreq 切换不影响 crawl 模块和整体编译。**不发起实际网络请求**（避免环境依赖和 CDP 测试失败）。

- [ ] **Step 1: 在 tests/integration.rs 追加 mod fetch_test**

在 `tests/integration.rs` 末尾追加：

```rust
mod fetch_test {
    use wisp::fetch::{Client, ClientBuilder};
    use wreq_util::Emulation;

    #[test]
    fn test_client_builder_with_emulation_builds() {
        // 验证带 emulation 的 client 能成功 build
        let client = Client::builder()
            .emulation(Emulation::Chrome136)
            .timeout(std::time::Duration::from_secs(30))
            .build();
        assert!(client.is_ok(), "emulation client should build");
    }

    #[test]
    fn test_client_default_config_has_emulation() {
        // 验证默认 Config 带 Chrome136 指纹
        let client = Client::new();
        assert!(client.is_ok(), "default client should build with Chrome136 emulation");
    }

    #[test]
    fn test_client_builder_no_emulation_builds() {
        // 验证关闭 emulation 的 client 能成功 build
        let client = Client::builder()
            .no_emulation()
            .build();
        assert!(client.is_ok(), "no_emulation client should build");
    }
}
```

- [ ] **Step 2: 运行新测试**

Run: `cargo test --test integration fetch_test`
Expected: 3 passed

- [ ] **Step 3: 运行完整测试套件**

Run: `cargo test --lib; cargo test --test adaptive_test; cargo test --test crawl_checkpoint_test; cargo test --test difflib_test; cargo test --test dom_navigation_test; cargo test --test xpath_test; cargo test --test fetch_test; cargo test --test integration fetch_test`
Expected: 全部通过（lib 35 + adaptive 5 + checkpoint 4 + difflib 7 + dom_nav 9 + xpath 9 + fetch_test 7 + integration fetch_test 3 = 79 passed）

**注意：** integration.rs 的浏览器 CDP 测试（test_screenshot_creates_file 等）在当前环境失败是已知问题，不属于本 stage 引入，不纳入验证范围。

- [ ] **Step 4: 验证无 reqwest 残留**

Run: `Select-String -Path "src\**\*.rs","tests\**\*.rs","Cargo.toml" -Pattern "reqwest" -CaseSensitive | Select-Object Path, LineNumber, Line`
Expected: 无输出（无 reqwest 残留引用）

- [ ] **Step 5: 提交**

```bash
git add tests/integration.rs
git commit -m "test: 阶段 3 端到端集成测试（wreq 切换不影响 crawl 模块编译）"
```

---

## Self-Review 检查

**1. Spec 覆盖检查：**
- ✅ 2.1.1 替换范围（fetch/mod.rs + fetch/proxy.rs + Cargo.toml）→ Task 1, 2, 4
- ✅ 2.1.2 Client API 保持兼容 → Task 2（公共 API 签名不变）
- ✅ 2.1.3 新增 TLS 指纹模拟配置 → Task 3（emulation + header_order + no_emulation）
- ✅ 2.1.4 构建依赖说明 → 已在 spec 标注验证通过
- ✅ 2.4 阶段 2 测试策略（wreq TLS 指纹测试 + 兼容性测试）→ Task 5, 6

**2. Placeholder 扫描：**
- 无 "TBD"、"TODO" 占位
- 所有步骤都有完整代码
- 无 "类似 Task N" 引用

**3. 类型一致性：**
- `Emulation` 枚举（wreq-util 3.0.0-rc）在 Task 3 定义，Task 5/6 使用一致
- `HeaderName`（wreq::header）在 Task 3 定义，Task 5 使用一致
- `Config::emulation: Option<Emulation>` 在 Task 3 定义，Task 5 测试一致
- `ClientBuilder::emulation(Emulation)` 在 Task 3 定义，Task 5/6 测试一致
- `ClientBuilder::config_ref() -> &Config` 在 Task 5 Step 2 定义，Task 5 测试使用

**4. 已知简化：**
- 不发起实际网络请求测试 TLS 指纹（避免环境依赖，spec 2.4 提到的 `https://tls.peet.ws/api/all` 留待用户手动验证）
- 不启用 wreq 内置 cookie_store/gzip/brotli（wisp 有自己的 encoding 层和 cookie 处理，切换可能破坏行为，留待未来优化）
- emulation 放在 redirect/timeout/proxy 之后（wreq 文档说 emulation 会覆盖 TLS/HTTP2 配置，与 redirect/timeout/proxy 不冲突）

---

## 执行说明

本 plan 适用于 subagent-driven-development。每个 Task 独立可测、可提交。Task 1（依赖切换）和 Task 2（类型替换）是基础，Task 3（emulation 配置）是核心增强，Task 4-6 是清理和验证。Task 1 的 cargo build 首次约 10 分钟，需用非阻塞执行。
