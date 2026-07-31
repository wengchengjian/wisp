### Task 10: 修复浏览器模式代理认证丢失

**Files:**
- Modify: `src/browser/launch.rs:95-98`（build_stealth_args proxy 段）
- Modify: `src/browser/mod.rs`（Browser 启动后注入代理认证，若 launch 不支持则记录）
- Test: `src/browser/launch.rs` 内 `#[cfg(test)]`

**Interfaces:**
- Consumes: `ProxyConfig { server, username, password }`，`Page::evaluate`（注入 JS 设置代理认证）
- Produces: `build_stealth_args` 仍只设 `proxy-server`（Chrome 限制），但启动后通过 CDP `Fetch.requestPaused` 或 JS 注入 `chrome.webRequest` 处理 407。鉴于实现复杂，此 task 采用文档化限制 + 启动日志告警。

**背景：** Chrome 的 `--proxy-server` 不支持内联认证。代理认证需通过 CDP 拦截 407 响应或扩展程序。完整实现超出本修复范围。本 task 采用务实方案：当配置了 username/password 时记录 warn 日志明确告知限制，避免静默丢失。

- [ ] **Step 1: 写测试 — 配置认证时记录告警（验证日志或行为）**

由于日志验证复杂，改为验证 `build_stealth_args` 在有认证时不崩溃且仍设 proxy-server。在 `src/browser/launch.rs` 的 `#[cfg(test)]` 末尾追加：

```rust
    #[test]
    fn test_stealth_args_proxy_with_auth_still_sets_server() {
        let opts = LaunchOptions {
            proxy: Some(crate::config::ProxyConfig {
                server: "http://127.0.0.1:8080".into(),
                username: Some("user".into()),
                password: Some("pass".into()),
            }),
            ..Default::default()
        };
        let args = build_stealth_args(&opts);
        // proxy-server 仍设置
        assert!(args.iter().any(|a| a == "proxy-server=http://127.0.0.1:8080"),
            "proxy-server 应设置");
    }
```

- [ ] **Step 2: 运行测试确认通过（现有实现已满足）**

Run: `cargo test --lib browser::launch::tests::test_stealth_args_proxy_with_auth_still_sets_server 2>&1 | tail -10`
Expected: PASS（现有实现已设 proxy-server，仅认证未应用）。

此测试验证不崩溃。告警逻辑在 Step 3 添加。

- [ ] **Step 3: 添加告警日志**

修改 `src/browser/launch.rs` 的 `build_stealth_args` proxy 段（L95-98）：

```rust
    // Proxy
    if let Some(ref proxy) = options.proxy {
        args.push(format!("proxy-server={}", proxy.server));
        // Chrome --proxy-server 不支持内联认证；username/password 无法通过命令行传递。
        // 需通过 CDP Fetch.requestPaused 拦截 407 或扩展程序处理（当前未实现）。
        if proxy.username.is_some() || proxy.password.is_some() {
            tracing::warn!(
                "Browser proxy auth (username/password) is not supported via --proxy-server. \
                 The proxy will be used without authentication; expect 407 responses. \
                 To use authenticated proxies with browser mode, configure the proxy to \
                 whitelist the client IP or use an unauthenticated proxy."
            );
        }
    }
```

- [ ] **Step 4: 编译并运行测试**

Run: `cargo build 2>&1 | tail -10`
Expected: 编译通过。

Run: `cargo test --lib browser::launch 2>&1 | tail -10`
Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add src/browser/launch.rs
git commit -m "fix(browser): 代理认证丢失改为显式告警

- Chrome --proxy-server 不支持内联认证，配置 username/password 时记录 warn
- 明确告知限制（需 CDP 407 拦截或 IP 白名单），避免静默丢失"
```

---

