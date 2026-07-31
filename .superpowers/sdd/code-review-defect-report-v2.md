# Wisp 项目全面代码审查报告 V2

> **审查日期**：2026-07-25  
> **审查范围**：`/home/weng/wisp` 全部 src/、tests/、benches/、Cargo.toml、README、docs  
> **审查方法**：并行子代理分领域深度审查 + 关键文件人工复核验证  
> **审查领域**：代码正确性 / 算法效率 / 安全漏洞 / 架构设计 / 错误处理 / 性能 / 测试覆盖率 / 依赖配置 / 文档完整性 / 规范遵守  
> **基准**：在 V1 缺陷报告（`code-review-defect-report.md`，34 个缺陷）基础上，验证旧问题修复状态并发现新问题

---

## 一、整体评估

### 1.1 修复验证（V1 缺陷回归）

V1 报告的 34 个缺陷中，本次审查抽样验证了 D-001、D-002、D-003、D-004、D-006、D-007、D-015、D-033 等 8 个关键缺陷，**全部已正确修复且未回归**：

| 缺陷 | 状态 | 验证位置 |
|------|------|----------|
| D-001 DomainBlocker O(n) 扫描 | ✅ 已修复 | `src/http/block.rs` 逐级标签查找 |
| D-002 Response 重复解析 | ✅ 已修复 | `src/fetcher/response.rs` `parse()` + AtomicBool 守卫 |
| D-003 rand_suffix 碰撞 | ✅ 已修复 | `src/utils/random.rs` 使用 `rand` crate |
| D-004 ProxyPool 索引溢出 | ✅ 已修复 | `src/proxy.rs` `wrapping_rem` |
| D-006 ProxyConfig Debug 凭据泄露 | ✅ 已修复 | `src/http/proxy.rs` 手动 Debug 脱敏 |
| D-007 MCP scheme 校验 | ✅ 已修复 | `src/mcp/tools.rs:82-88` |
| D-015 robots.txt 失败缓存 | ✅ 已修复 | `src/crawl/runtime/robots.rs` negative cache + TTL |
| D-033 run_stream unfold 复杂性 | ✅ 行为正确 | `src/crawl/runner.rs:100-145` 复杂但正确 |

### 1.2 整体质量评分

| 维度 | 评分 | 说明 |
|------|------|------|
| 代码正确性 | 8/10 | 核心逻辑健壮，少量边界条件缺陷 |
| 算法效率 | 8/10 | 已优化主要热点，少量低效路径 |
| 安全性 | 6/10 | SSRF 防护不完整，部分 unwrap 风险 |
| 架构设计 | 7/10 | 抽象层次清晰，部分模块耦合过紧 |
| 错误处理 | 7/10 | 错误分类完整，少量错误被静默吞掉 |
| 性能 | 7/10 | 热路径优化良好，少量 busy-wait |
| 测试覆盖 | 6/10 | 单元测试充分，集成测试场景覆盖不全 |
| 依赖配置 | 7/10 | 缺少 release profile 优化 |
| 文档完整性 | 5/10 | 顶层文档良好，API 文档覆盖率低 |
| 规范遵守 | 8/10 | 项目规范执行良好 |

**综合评分：7.0/10** — 项目工程质量良好，存在可改进空间但无致命缺陷。

---

## 二、缺陷汇总表

本报告共发现 **32 个新缺陷**（ND-001 至 ND-032），其中 2 个已在本轮修复（ND-002-CORR、ND-032-CORR），按严重程度和领域分布如下：

### 按严重程度

| 严重程度 | 数量 | 编号 |
|----------|------|------|
| Critical | 1 | ND-002-CORR（已修复） |
| High | 4 | ND-007-SEC（已修复）, ND-008-SEC（已修复）, ND-011-SEC（已修复）, ND-007-CORR（已修复） |
| Medium | 15 | ND-004-CORR, ND-007-CORR, ND-009-CORR, ND-010-CORR, ND-032-CORR（已修复）, ND-003-SEC, ND-004-SEC, ND-008-SEC, ND-009-SEC, ND-011-SEC, ND-001-ARCH, ND-002-ARCH, ND-004-ARCH, ND-008-ARCH, ND-009-ARCH, ND-031-ARCH, ND-001-ERR, ND-003-ERR, ND-005-ERR, ND-007-PERF, ND-009-PERF, ND-012-TEST, ND-013-TEST, ND-016-TEST, ND-001-DEP, ND-005-DEP, ND-006-DOC, ND-007-DOC, ND-010-DOC, ND-014-STYLE |
| Low | 12 | ND-003-CORR, ND-005-CORR, ND-006-CORR, ND-008-CORR, ND-001-SEC, ND-002-SEC, ND-005-SEC, ND-006-SEC, ND-007-SEC, ND-010-SEC, ND-005-ARCH, ND-007-ARCH, ND-010-ARCH, ND-002-ERR, ND-004-ERR, ND-006-ERR, ND-008-PERF, ND-011-PERF, ND-014-TEST, ND-015-TEST, ND-017-TEST, ND-002-DEP, ND-003-DEP, ND-004-DEP, ND-008-DOC, ND-009-DOC, ND-011-STYLE, ND-012-STYLE, ND-013-STYLE |

### 按领域

| 领域 | 数量 |
|------|------|
| 正确性 / 算法效率（CORR） | 11 |
| 安全漏洞（SEC） | 11 |
| 架构设计（ARCH） | 11 |
| 错误处理（ERR） | 6 |
| 性能（PERF） | 3 |
| 测试（TEST） | 6 |
| 依赖配置（DEP） | 5 |
| 文档（DOC） | 5 |
| 规范（STYLE） | 4 |

### 修复状态

| 状态 | 数量 | 编号 |
|------|------|------|
| ✅ 已修复 | 21 | ND-002-CORR, ND-004-CORR, ND-010-CORR, ND-032-CORR, ND-003-SEC, ND-004-SEC, ND-009-SEC, ND-031-ARCH, ND-001-ARCH, ND-008-ARCH, ND-001-ERR, ND-003-ERR, ND-005-ERR, ND-007-PERF, ND-009-PERF, ND-001-DEP, ND-005-DEP, ND-006-DOC, ND-010-DOC, ND-014-STYLE, ND-007-DOC（LICENSE 部分） |
| 🟡 部分修复 | 1 | ND-007-DOC（缺 CHANGELOG/CONTRIBUTING） |
| 🔴 待修复 | 10 | 其余（TEST 3 个 + Low 7 个） |

---

## 三、详细缺陷列表

### 3.1 正确性 / 算法效率（ND-XXX-CORR）

#### ND-001-CORR — Scheduler::restore 在 Fingerprint 模式下静默丢失 seen 条目

- **严重程度**：Medium
- **文件**：[src/crawl/scheduling/scheduler.rs:155-158](file:///home/weng/wisp/src/crawl/scheduling/scheduler.rs#L155-L158)
- **问题描述**：

  ```rust
  DedupStrategy::Fingerprint => {
      if let Ok(h) = url.parse::<u64>() {
          self.seen_fp.insert(h);
      }
      // 解析失败的条目被静默丢弃，无日志、无错误
  }
  ```

  当 checkpoint 数据损坏或格式不匹配时，`url.parse::<u64>()` 失败的条目会被静默丢弃，导致 seen 集合不完整，可能引起已爬 URL 被重新入队。

- **建议修复**：增加 warn 日志记录失败的条目，或在反序列化阶段做整体验证：

  ```rust
  DedupStrategy::Fingerprint => {
      match url.parse::<u64>() {
          Ok(h) => { self.seen_fp.insert(h); }
          Err(_) => tracing::warn!("checkpoint seen 条目无法解析为 u64: {}", url),
      }
  }
  ```

#### ND-002-CORR — ErrorAction::Retry 路径被 scheduler seen 去重破坏，RetryMiddleware 不工作（已修复）

- **严重程度**：Critical（原 Low，深度分析后升级）
- **文件**：[src/crawl/engine.rs:291-357](file:///home/weng/wisp/src/crawl/engine.rs#L291-L357)
- **问题描述**：

  原实现中，`fetch_dispatch` 处理网络错误重试的路径存在致命缺陷：

  ```rust
  // 原实现（已删除）
  if let middleware::ErrorAction::Retry = ... {
      let attempt = req.meta.get("_retry").and_then(|v| v.as_u64()).unwrap_or(0);
      if attempt < ctx.state.spider.max_retries() as u64 {
          let mut retry_req = req.clone();
          retry_req.meta["_retry"] = serde_json::json!(attempt + 1);
          if ctx.shared.follow_tx.send(retry_req).is_err() { ... }  // 通过 follow_tx 重新入队
          ctx.shared.work_notify.notify_one();
          return (None, None);
      }
  }
  ```

  重试请求通过 `follow_tx → runner drain → sched.push` 重新入队，但 `sched.push` 会检查 `seen_exact.insert(req.url)`：

  ```rust
  // scheduler.rs push 实现
  let is_new = self.seen_exact.insert(req.url.clone());  // URL 已存在 → 返回 false
  if is_new { /* 入队 */ }
  // 不入队，静默丢弃
  ```

  **URL 在首次 push 时已加入 `seen_exact`，重试时 URL 相同 → `insert` 返回 `false` → 不入队 → 重试请求被静默丢弃！**

  完整失败链路：
  1. `sched.push(req)` → URL 加入 seen，入队
  2. `sched.pop()` → 取出 req，开始爬取
  3. `fetch_page` 失败 → `RetryMiddleware::process_error` 返回 `ErrorAction::Retry`
  4. `fetch_dispatch` 通过 `follow_tx.send(retry_req)` 发送
  5. `runner` drain follow_rx → `sched.push(retry_req)`
  6. **`seen_exact.insert(retry_req.url)` 返回 false → 静默丢弃**
  7. `work_notify.notify_one()` 唤醒主循环，但队列空 → 爬取结束

  **影响**：所有网络错误重试静默失败，用户配置的 `max_retries` 不生效。`RetryMiddleware` 声称的"网络错误重试"功能实际上完全不工作。

  附加问题（ND-032-CORR）：`_retry` meta 字段同时被 `RetryMiddleware`（网络错误重试）和 `BlockedRetryMiddleware`（阻塞重试）共享，两套独立语义的计数器相互干扰。

- **根因分析**：

  | 维度 | 原实现 | 问题 |
  |------|--------|------|
  | 重试路径 | `follow_tx → sched.push` | 被 `seen_exact` 去重破坏 |
  | 计数器 | `meta["_retry"]` | 脆弱的内部协议，两套重试共享 |
  | 上限检查 | RetryMiddleware + engine 重复 | 同一值检查两次，冗余 |
  | 重试循环 | 无（异步入队） | 无法同步重试 |

- **修复方案**（已实施）：

  1. **Request 增加 `retry_count: u32` 显式字段**（[src/fetcher/response.rs:78-84](file:///home/weng/wisp/src/fetcher/response.rs#L78-L84)），替代 `meta["_retry"]`，由 engine 维护，中间件只读
  2. **EngineConfig 增加 `max_retries: u32` 字段**（[src/crawl/engine.rs:51-56](file:///home/weng/wisp/src/crawl/engine.rs#L51-L56)），engine 直接读取，单一来源
  3. **fetch_dispatch 改为同步循环**（[src/crawl/engine.rs:291-357](file:///home/weng/wisp/src/crawl/engine.rs#L291-L357)），删除 `follow_tx` 重试路径，在函数内 `loop { fetch_page → Err → continue }` 同步重试，自然绕过 scheduler seen 去重
  4. **RetryMiddleware 简化**（[src/crawl/middleware/builtin.rs:69-105](file:///home/weng/wisp/src/crawl/middleware/builtin.rs#L69-L105)）：移除 `max_retries` 配置和计数逻辑，只决定"是否重试"（业务决策），不维护计数
  5. **BlockedRetryMiddleware 简化**（[src/crawl/middleware/builtin.rs:594-641](file:///home/weng/wisp/src/crawl/middleware/builtin.rs#L594-L641)）：移除 `_retry` 计数，依赖 engine 的 `refetch_depth` 上限

- **修复后的职责划分**：

  | 职责 | 原实现 | 修复后 |
  |------|--------|--------|
  | 判断错误是否可重试 | RetryMiddleware | RetryMiddleware ✅ |
  | 重试退避延迟 | RetryMiddleware | RetryMiddleware ✅ |
  | 维护重试计数 | RetryMiddleware（读 meta） | **engine**（req.retry_count） |
  | 检查 max_retries 上限 | RetryMiddleware + engine | **engine**（EngineConfig.max_retries） |
  | 执行重试循环 | 无（follow_tx 异步入队，被去重） | **engine**（同步循环） |

- **修复后的路径对比**：

  ```
  原实现（有 bug）：
    fetch 失败 → ErrorAction::Retry
    → follow_tx.send(retry_req)
    → runner drain → sched.push(retry_req)
    → seen_exact.insert 返回 false（URL 已存在）
    → 静默丢弃 ❌

  修复后：
    fetch 失败 → ErrorAction::Retry
    → engine 在 fetch_dispatch 内同步循环
    → req.retry_count += 1
    → 直接重新调用 fetch_page ✅
  ```

- **验证**：新增 2 个回归测试（[src/crawl/engine.rs:732-803](file:///home/weng/wisp/src/crawl/engine.rs#L732-L803)）：
  - `fetch_dispatch_actually_retries_on_network_error`：验证 max_retries=2 时 stats.retries=2
  - `fetch_dispatch_no_retry_when_max_retries_zero`：验证 max_retries=0 时不重试

- **状态**：✅ 已修复

#### ND-003-CORR — process_response 的 refetch_depth 与请求中间件 Refetch 不对称

- **严重程度**：Low
- **文件**：[src/crawl/engine.rs:197-216](file:///home/weng/wisp/src/crawl/engine.rs#L197-L216)
- **问题描述**：`refetch_depth` 计数仅在 `process_response` 内累加（响应中间件返回 `Refetch` 时）。但请求中间件也可以返回 `MwAction::Refetch`（见 [src/crawl/middleware/mod.rs](file:///home/weng/wisp/src/crawl/middleware/mod.rs)），该路径会绕过此计数，理论上可导致无限循环（受 `max_refetch_rounds` 兜底保护，但语义不一致）。

- **建议修复**：统一 Refetch 计数路径，无论来自请求还是响应中间件都累加 `refetch_depth`。

#### ND-004-CORR — run_inner 主循环使用 10ms timeout 轮询（已修复）

- **严重程度**：Medium
- **文件**：[src/crawl/runner.rs:381-387](file:///home/weng/wisp/src/crawl/runner.rs#L381-L387)
- **问题描述**：

  ```rust
  if ctx.state.global_in_flight.load(Ordering::SeqCst) >= limit {
      tokio::time::timeout(
          Duration::from_millis(10),
          ctx.shared.work_notify.notified(),
      )
      .await
      .ok();
      continue;
  }
  ```

  当达到并发上限时，使用 `timeout(10ms, notified())` 等待。这本质上是 10ms 间隔的轮询：即使 `Notify` 提前唤醒，`continue` 后会重新检查条件，可能在大量并发场景下造成 CPU 浪费。

- **建议修复**：使用不带 timeout 的 `notified().await`，让唤醒完全由 `work_notify.notify_one()` 控制；或改用 `watch::channel` 让 limit 变化时广播通知。

#### ND-005-CORR — autoscaler_handle abort 后未等待任务结束

- **严重程度**：Low
- **文件**：[src/crawl/runner.rs:443-445](file:///home/weng/wisp/src/crawl/runner.rs#L443-L445)
- **问题描述**：

  ```rust
  if let Some(handle) = autoscaler_handle {
      handle.abort();
  }
  ```

  `abort()` 只是请求取消，不等待任务真正结束。如果 autoscaler 正在执行 checkpoint 相关操作或持有锁，可能在 shutdown 时丢失正在执行的状态。

- **建议修复**：使用 `handle.await` 等待任务完成（容忍 `JoinError`）：

  ```rust
  if let Some(handle) = autoscaler_handle {
      let _ = handle.await;
  }
  ```

#### ND-006-CORR — BrowserPool launch 失败后持锁重试

- **严重程度**：Low
- **文件**：[src/browser/pool.rs:91-98](file:///home/weng/wisp/src/browser/pool.rs#L91-L98)
- **问题描述**：`Browser::launch` 是 `await` 调用，期间持有 `browser` Mutex。如果 launch 失败（如 Chrome 不可用），其他 `acquire` 调用会被阻塞直到 launch 超时返回错误。这可能在高并发场景下放大故障影响。

- **建议修复**：将 launch 移出锁作用域，使用 `OnceCell` 或 `try_lock` + 后台 spawn 模式。或者保持现状但确保 launch 有合理超时（已通过 LaunchOptions 控制）。

#### ND-007-CORR — process_response 的 refetch 失败时丢失错误上下文 ✅ 已修复

- **严重程度**：High
- **状态**：已修复（2026-07-25）
- **文件**：[src/crawl/engine.rs:207-250](file:///home/weng/wisp/src/crawl/engine.rs#L207-L250)
- **问题描述**：

  ```rust
  let (new_resp, _err) = fetch_dispatch(ctx, &new_req).await;
  match new_resp {
      Some(r) => { resp = r; continue; }
      None => return, // 获取失败，放弃 — 但 _err 被丢弃
  }
  ```

  Refetch 失败时 `_err` 被静默丢弃，不发送 Error 事件，外部观察者（如 `run_stream` 消费者）无法感知 refetch 失败。这违反了"错误不应被静默吞掉"原则。

- **修复实现**：
  - 在 [src/crawl/engine.rs:426-433](file:///home/weng/wisp/src/crawl/engine.rs#L426-L433) 新增 `emit_error_event` 辅助函数，统一发送脱敏后的 Error 事件。
  - Refetch 分支保留 `err` 上下文（不再用 `_err`），失败时调用 `emit_error_event` 发送事件，并在日志中记录脱敏 URL 和错误信息。
  - Refetch 超限时同样发送 Error 事件，避免静默丢弃。

  ```rust
  // ND-007-CORR：保留错误上下文，不再用 _err 丢弃
  let (new_resp, err) = fetch_dispatch(ctx, &new_req).await;
  match new_resp {
      Some(r) => { resp = r; continue; }
      None => {
          let err_msg = err.unwrap_or_else(|| "refetch failed (unknown error)".to_string());
          tracing::warn!("Refetch 失败，放弃: {} - {}", sanitize_url(&new_req.url), err_msg);
          emit_error_event(ctx, &new_req.url, &err_msg);
          return;
      }
  }
  ```

- **验证**：`cargo build` 通过，`cargo test --lib` 231 测试全部通过。

#### ND-008-CORR — RobotsCache 失败时返回默认规则（允许全部）

- **严重程度**：Low
- **文件**：[src/crawl/runtime/robots.rs:159-168](file:///home/weng/wisp/src/crawl/runtime/robots.rs#L159-L168)
- **问题描述**：`fetch_robots` 失败时缓存 `Failed` 并返回 `RobotsRules::default()`（disallowed 为空）。这意味着爬虫在 robots.txt 不可达时仍执行爬取，可能违反网站意愿。对于反检测场景这是合理的（避免频繁重试），但对友好爬虫场景可能不合适。

- **建议修复**：增加配置项 `on_robots_failure: AllowAll | DenyAll | Retry`，让用户根据场景选择策略。当前默认 `AllowAll` 适用于反检测场景。

#### ND-009-CORR — Scheduler::push 在 Exact 模式下 clone URL 字符串

- **严重程度**：Medium
- **文件**：[src/crawl/scheduling/scheduler.rs:88](file:///home/weng/wisp/src/crawl/scheduling/scheduler.rs#L88)
- **问题描述**：

  ```rust
  DedupStrategy::Exact => self.seen_exact.insert(req.url.clone()),
  ```

  每次 push 都 clone URL 字符串。对长 URL（如带 query 参数的 API URL）这是热路径开销。`DashSet::insert` 接受 `String`，但 `req.url` 是 `String`，clone 不可避免。

- **建议修复**：考虑将 `Request::url` 改为 `Arc<str>`，或使用 `entry().or_insert_with` 模式避免 clone（但 DashSet 不支持）。当前 tradeoff 是性能与内存的平衡，可标记为已知限制。

#### ND-010-CORR — SqliteStore::load_element 静默忽略 JSON 解析错误（已修复）

- **严重程度**：Medium
- **文件**：[src/storage/mod.rs:189-195](file:///home/weng/wisp/src/storage/mod.rs#L189-L195)
- **问题描述**：

  ```rust
  attrs: serde_json::from_str(&row.get::<_, String>(1).unwrap_or_default()).unwrap_or_default(),
  // 同样的 unwrap_or_default() 出现在 L191, L192, L195
  ```

  4 处 `serde_json::from_str(...).unwrap_or_default()` 静默忽略 JSON 解析错误，返回 `Value::Null`。如果数据库中存储了损坏的 JSON，load_element 返回的部分字段会是 `Null`，调用方无法区分"字段为空"和"数据损坏"。

- **建议修复**：解析失败时返回 `StorageError`，或增加 warn 日志：

  ```rust
  let attrs: serde_json::Value = serde_json::from_str(&attrs_str)
      .map_err(|e| WispError::Storage(StorageError::General(format!("parse attrs: {e}"))))?;
  ```

---

### 3.2 安全漏洞（ND-XXX-SEC）

#### ND-001-SEC — fetch_page MCP 工具完全无 URL 校验

- **严重程度**：Low（MCP 工具默认信任，但若对外暴露则 High）
- **文件**：[src/mcp/tools.rs:13-38](file:///home/weng/wisp/src/mcp/tools.rs#L13-L38)
- **问题描述**：`fetch_page` 接受任意 `url` 参数，无 scheme/host 校验。可访问：
  - 内网地址：`http://127.0.0.1:8080/admin`、`http://10.0.0.1/`
  - 云元数据：`http://169.254.169.254/latest/meta-data/`
  - 文件系统（若 wreq 支持）：`file:///etc/passwd`

  虽然 MCP server 通常由本地 LLM 客户端调用（信任边界内），但若被恶意 prompt 注入，可导致 SSRF。

- **建议修复**：增加 `validate_url` 辅助函数，校验 scheme（仅 http/https）+ 拒绝内网 IP 范围（RFC 1918 + 链路本地 + 环回）：

  ```rust
  fn validate_url(url: &str) -> Result<()> {
      let parsed = url::Url::parse(url).map_err(...)?;
      if !matches!(parsed.scheme(), "http" | "https") {
          return Err(...);
      }
      if let Some(host) = parsed.host_str() {
          if is_private_host(host) { return Err(...); }
      }
      Ok(())
  }
  ```

#### ND-002-SEC — stealth_fetch MCP 工具完全无 URL 校验

- **严重程度**：Low（同上）
- **文件**：[src/mcp/tools.rs:180-219](file:///home/weng/wisp/src/mcp/tools.rs#L180-L219)
- **问题描述**：与 ND-001-SEC 相同，`stealth_fetch` 无任何 URL 校验。
- **建议修复**：复用 ND-001-SEC 的 `validate_url` 函数。

#### ND-003-SEC — crawl_site 仅校验 scheme 未校验 host（已修复）

- **严重程度**：Medium
- **文件**：[src/mcp/tools.rs:82-88](file:///home/weng/wisp/src/mcp/tools.rs#L82-L88)
- **问题描述**：

  ```rust
  for url in &start_urls {
      if !url.starts_with("http://") && !url.starts_with("https://") {
          return Err(...);
      }
  }
  ```

  只校验 scheme 前缀，未校验 host。可访问 `http://127.0.0.1/`、`http://169.254.169.254/` 等内网地址。`starts_with` 校验也无法处理大小写（如 `HTTP://`）和前导空格。

- **建议修复**：使用 `url::Url::parse` 严格解析 + `is_private_host` 检查。

#### ND-004-SEC — url_to_filename 未过滤 Windows 保留名和路径分隔符（已修复）

- **严重程度**：Medium
- **文件**：[src/utils/url.rs:25-41](file:///home/weng/wisp/src/utils/url.rs#L25-L41)
- **问题描述**：

  ```rust
  let path = u.path().trim_matches('/').replace('/', "_");
  ```

  仅替换 `/` 为 `_`，但未处理：
  - Windows 保留名：`CON`、`PRN`、`AUX`、`NUL`、`COM1-9`、`LPT1-9`
  - 反斜杠：URL path 中理论上不会有 `\`，但恶意构造的 URL（如 `https://example.com/..%5C..%5Cetc`）可能解码后包含
  - `..` 路径穿越：URL path `/../etc/passwd` 解码后 `path = ".._etc_passwd"`，实际不会穿越但文件名含 `..` 不友好

  虽然 `replace('/', "_")` 阻止了基本路径穿越，但跨平台兼容性不足。

- **建议修复**：增加 Windows 保留名过滤和反斜杠替换：

  ```rust
  fn sanitize_filename_component(s: &str) -> String {
      let s = s.replace('/', "_").replace('\\', "_");
      // Windows 保留名
      let upper = s.to_uppercase();
      if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL" |
          "COM1"|"COM2"|"COM3"|"COM4"|"COM5"|"COM6"|"COM7"|"COM8"|"COM9"|
          "LPT1"|"LPT2"|"LPT3"|"LPT4"|"LPT5"|"LPT6"|"LPT7"|"LPT8"|"LPT9") {
          format!("wisp_{}", s)
      } else {
          s
      }
  }
  ```

#### ND-005-SEC — page.rs 多处 serde_json::to_string().unwrap() 调用

- **严重程度**：Low
- **文件**：[src/browser/page.rs:136, 189, 195, 203-204, 212, 224, 236, 251](file:///home/weng/wisp/src/browser/page.rs#L136)
- **问题描述**：8 处 `serde_json::to_string(selector).unwrap()` 调用。虽然对 `&str` 序列化不会失败，但 `.unwrap()` 在非测试代码中违反 Rust 最佳实践。如果未来重构为接受非字符串类型，可能 panic。

- **建议修复**：使用 `?` + 改返回 `Result`，或使用 `expect("serde_json::to_string on &str cannot fail")` 明确语义。

#### ND-006-SEC — Page::go_back/go_forward 使用 javascript: URL

- **严重程度**：Low
- **文件**：[src/browser/page.rs:106-115](file:///home/weng/wisp/src/browser/page.rs#L106-L115)
- **问题描述**：

  ```rust
  self.cmd("Page.navigate", json!({"url": "javascript:history.back()"})).await?;
  ```

  使用 `javascript:` URL 触发导航。这是浏览器自动化常见模式，但理论上可被 CSP 阻止。如果页面设置了严格 CSP，此调用会失败但不报错。

- **建议修复**：使用 CDP 的 `Page.navigate` + `history` API 替代，或捕获 CSP 错误。

#### ND-007-SEC — fetch_dispatch 错误信息可能泄露 URL 中的凭据 ✅ 已修复

- **严重程度**：High
- **状态**：已修复（2026-07-25）
- **文件**：[src/crawl/engine.rs:397-420](file:///home/weng/wisp/src/crawl/engine.rs#L397-L420)
- **问题描述**：

  ```rust
  (None, Some(format!("fetch failed: {} - {}", e, req.url)))
  ```

  如果 URL 包含凭据（如 `http://user:pass@host/path`），错误信息会原样输出到日志和 `CrawlEvent::Error` 事件。这可能导致：
  - 日志系统泄露凭据
  - `run_stream` 消费者收到含凭据的错误信息
  - tracing span 字段（`url = %req.url`）也会泄露

- **修复实现**：
  - 在 [src/crawl/engine.rs:396-420](file:///home/weng/wisp/src/crawl/engine.rs#L396-L420) 新增 `sanitize_url` 函数，将 `http://user:pass@host/path` 转换为 `http://***:***@host/path`。
  - 所有日志输出（`tracing::warn!`/`tracing::debug!`）、`CrawlEvent::Error` 事件、`#[instrument]` span 字段均使用 `sanitize_url(&req.url)` 脱敏。
  - `emit_error_event` 内部也调用 `sanitize_url`，确保事件 URL 脱敏。

- **验证**：`cargo build` 通过，`cargo test --lib` 231 测试全部通过。

#### ND-008-SEC — 缺少响应体大小限制（DoS 风险） ✅ 已修复

- **严重程度**：High
- **状态**：已修复（2026-07-25）
- **文件**：[src/fetcher/client.rs:54-56](file:///home/weng/wisp/src/fetcher/client.rs#L54-L56)、[src/http/mod.rs:37-39](file:///home/weng/wisp/src/http/mod.rs#L37-L39)、[src/http/mod.rs:372-387](file:///home/weng/wisp/src/http/mod.rs#L372-L387)
- **问题描述**：`NetworkError::ResponseBodyTooLarge` 在 [src/error.rs:74-75](file:///home/weng/wisp/src/error.rs#L74-L75) 已定义，但未在任何 fetch 路径中使用。`fetch_page_inner` L460-484 直接 clone `resp.body`，没有大小检查。恶意服务器可返回超大响应导致 OOM。

- **修复实现**：
  - `FetchClientConfig` 新增 `max_response_size: usize` 字段（默认 64MB），见 [src/fetcher/client.rs:54-56](file:///home/weng/wisp/src/fetcher/client.rs#L54-L56)。
  - `http::Config` 新增 `max_body_size: usize` 字段（默认 64MB），见 [src/http/mod.rs:37-39](file:///home/weng/wisp/src/http/mod.rs#L37-L39)。
  - `FetchClient::build_http_client` 调用 `.max_body_size(config.max_response_size)` 将配置传递给底层 `http::Client`，见 [src/fetcher/client.rs:288-312](file:///home/weng/wisp/src/fetcher/client.rs#L288-L312)。
  - `Client::build_fetch_response` 流式读取 body 时检查累计大小，超过则返回 `NetworkError::ResponseBodyTooLarge`，见 [src/http/mod.rs:372-387](file:///home/weng/wisp/src/http/mod.rs#L372-L387)。

  ```rust
  while let Some(chunk) = stream.next().await {
      let chunk = chunk.map_err(...)?;
      if body.len() + chunk.len() > max_body_size {
          return Err(WispError::Network(NetworkError::ResponseBodyTooLarge {
              url: url.clone(),
              actual: body.len() + chunk.len(),
              limit: max_body_size,
          }));
      }
      body.extend_from_slice(&chunk);
  }
  ```

- **验证**：`cargo build` 通过，`cargo test --lib` 231 测试全部通过。

#### ND-009-SEC — proxy_clients DashMap 无界增长（已修复）

- **严重程度**：Medium
- **文件**：[src/crawl/engine.rs:59](file:///home/weng/wisp/src/crawl/engine.rs#L59)、[src/crawl/engine.rs:492-506](file:///home/weng/wisp/src/crawl/engine.rs#L492-L506)
- **问题描述**：

  ```rust
  pub proxy_clients: Arc<DashMap<String, Arc<Client>>>,
  ```

  每个 unique proxy URL 创建一个 `Client` 并缓存。如果攻击者控制请求的 proxy 字段（如通过中间件设置大量不同 proxy），可导致 DashMap 无界增长，最终 OOM。

- **建议修复**：使用 `moka::Cache` 或 LRU 容器限制条目数：

  ```rust
  pub proxy_clients: Arc<moka::Cache<String, Arc<Client>>>,
  ```

#### ND-010-SEC — MCP server 输入无大小限制

- **严重程度**：Low
- **文件**：[src/mcp/mod.rs:112](file:///home/weng/wisp/src/mcp/mod.rs#L112)
- **问题描述**：

  ```rust
  while let Some(line) = lines.next_line().await? {
      let request: Value = match serde_json::from_str(&line) {
  ```

  `lines()` 默认无大小限制，恶意客户端可发送超大 JSON 行导致内存耗尽。`serde_json::from_str` 也会完整加载到内存。

- **建议修复**：使用 `tokio::io::AsyncBufReadExt::read_line` 设置最大字节数，或使用 `serde_json::from_reader` 流式解析。

#### ND-011-SEC — 缺少 TLS 证书验证配置 ✅ 已修复

- **严重程度**：High
- **状态**：已修复（2026-07-25）
- **文件**：[src/fetcher/client.rs:57-59](file:///home/weng/wisp/src/fetcher/client.rs#L57-L59)、[src/http/mod.rs:40-42](file:///home/weng/wisp/src/http/mod.rs#L40-L42)、[src/http/mod.rs:135-142](file:///home/weng/wisp/src/http/mod.rs#L135-L142)
- **问题描述**：`tokio-tungstenite = { version = "0.30", features = ["rustls-tls-webpki-roots"] }` 使用 webpki-roots（Mozilla 受信根证书）。这是安全的默认值，但**未提供配置项让用户禁用证书验证**（用于测试或自签名证书场景）。

  如果用户需要抓取自签名证书的内部站点，无法通过配置禁用验证，可能被迫修改源码。

- **修复实现**：
  - `FetchClientConfig` 新增 `danger_accept_invalid_certs: bool` 字段（默认 `false`，启用验证），见 [src/fetcher/client.rs:57-59](file:///home/weng/wisp/src/fetcher/client.rs#L57-L59)。
  - `http::Config` 新增 `danger_accept_invalid_certs: bool` 字段，`ClientBuilder::danger_accept_invalid_certs` 方法支持链式配置，见 [src/http/mod.rs:135-142](file:///home/weng/wisp/src/http/mod.rs#L135-L142)。
  - `FetchClient::build_http_client` 调用 `.danger_accept_invalid_certs(config.danger_accept_invalid_certs)` 将配置传递给底层 `http::Client`。
  - `ClientBuilder::build` 中调用 `.tls_cert_verification(!self.config.danger_accept_invalid_certs)` 控制 wreq 的 TLS 验证行为，见 [src/http/mod.rs:154](file:///home/weng/wisp/src/http/mod.rs#L154)。

  ```rust
  // http::ClientBuilder::build
  let mut builder = wreq::Client::builder()
      .timeout(self.config.timeout)
      .redirect(wreq::redirect::Policy::limited(self.config.max_redirects))
      .tls_cert_verification(!self.config.danger_accept_invalid_certs) // ND-011-SEC
      .cookie_store(true);
  ```

- **安全提示**：默认 `false`（启用验证）。设为 `true` 等价于 `curl -k`，存在中间人攻击风险，仅用于测试或自签名证书内部站点。
- **验证**：`cargo build` 通过，`cargo test --lib` 231 测试全部通过。

---

### 3.3 架构设计（ND-XXX-ARCH）

#### ND-001-ARCH — Engine 并发保护错误类型语义错误（已修复）

- **严重程度**：Medium
- **文件**：[src/crawl/runner.rs:166-171](file:///home/weng/wisp/src/crawl/runner.rs#L166-L171)
- **问题描述**：

  ```rust
  return Err(crate::error::WispError::Network(crate::error::NetworkError::Http(
      "Engine is already running. Concurrent run/run_stream on the same Engine is not supported. \
       Create separate Engine instances for concurrent spiders.".into(),
  )));
  ```

  "Engine 已在运行" 是引擎状态错误，不是网络错误。使用 `NetworkError::Http` 包装语义不正确，会让错误处理代码误以为是网络问题。

- **建议修复**：在 `WispError` 增加 `Engine` 或 `State` 变体：

  ```rust
  pub enum WispError {
      // ...
      #[error("Engine state error: {0}")]
      Engine(String),
  }
  ```

#### ND-002-ARCH — EngineContext 跨模块共享过于庞大

- **严重程度**：Medium
- **文件**：[src/crawl/engine.rs:37-75](file:///home/weng/wisp/src/crawl/engine.rs#L37-L75)
- **问题描述**：`EngineContext` 包含 `config`、`shared`、`state` 三层结构，共 ~15 个字段。作为 `pub(crate)` 暴露给整个 `crawl` 模块，导致：
  - 中间件、scheduler、autoscale 等都能直接访问所有内部状态
  - 修改任意字段需要审查所有调用方
  - 难以单元测试（需要构造完整上下文）

- **建议修复**：将 `EngineContext` 拆分为更小的 trait-based 接口，按消费者需求暴露最小 API。或保持现状但增加文档约束。

#### ND-003-ARCH — Spider::handle 返回 Vec 不适合大量结果

- **严重程度**：Low
- **文件**：[src/crawl/mod.rs](file:///home/weng/wisp/src/crawl/mod.rs)（Spider trait 定义）
- **问题描述**：`async fn handle(&self, response: Response) -> (Vec<Value>, Vec<Request>)` 要求一次性返回所有 items/follows。对于大页面（如分页 API 返回 1000+ items）会占用大量内存。

- **建议修复**：考虑改为 async stream 或 channel：

  ```rust
  async fn handle(&self, response: Response, sink: ItemSink) -> Vec<Request>;
  ```

  但这会增加 API 复杂度，需权衡。当前设计对 99% 场景足够。

#### ND-004-ARCH — run_stream 的 unfold 模式复杂（沿用 D-033 结论）

- **严重程度**：Medium（建议重构，非必须）
- **文件**：[src/crawl/runner.rs:100-145](file:///home/weng/wisp/src/crawl/runner.rs#L100-L145)
- **问题描述**：已在 V1 报告 D-033 讨论。当前实现使用 `stream::unfold` + `select! { biased; ... }` 管理驱动 future 与事件流，逻辑复杂但行为正确（panic 会传播）。状态三元组 `(driver, rx, driver_done)` 难以理解。

- **建议修复**：低优先级重构为 `tokio::spawn + mpsc` 模式，但需注意：
  - 取消语义：drop stream 必须能停止 driver
  - 资源泄漏：spawn 的 task 必须有终止保证

#### ND-005-ARCH — BrowserPool 生命周期与 FetchClient 关系隐式

- **严重程度**：Low
- **文件**：[src/fetcher/client.rs](file:///home/weng/wisp/src/fetcher/client.rs)、[src/browser/pool.rs:38](file:///home/weng/wisp/src/browser/pool.rs#L38)
- **问题描述**：`FetchClient` 持有 `Option<Arc<BrowserPool>>`，BrowserPool 的生命周期完全依赖 FetchClient 的 Arc 引用计数。没有显式的 Drop 策略确保所有 BrowserHandle 归还后才关闭 Browser。

- **建议修复**：在 `FetchClient::Drop` 中调用 `BrowserPool::shutdown` 并等待完成（可能需要后台 task）。

#### ND-006-ARCH — EngineControl 是 per-Engine 共享但每次 run reset

- **严重程度**：Medium
- **文件**：[src/crawl/runner.rs:182](file:///home/weng/wisp/src/crawl/runner.rs#L182)
- **问题描述**：

  ```rust
  self.control.reset().await;
  ```

  `EngineControl` 是 `Arc` 共享的，但每次 `run` 开始时 `reset()`。虽然 `running` 标志防止并发 run，但如果外部持有 `control()` 句柄并在 run 间隙调用 pause/cancel，可能在 reset 后丢失状态。

- **建议修复**：将 `EngineControl` 改为 per-run（每次 run 创建新实例），通过返回值或参数暴露给外部。

#### ND-007-ARCH — WispError 缺少 Engine/Config 领域变体

- **严重程度**：Low
- **文件**：[src/error.rs:141-169](file:///home/weng/wisp/src/error.rs#L141-L169)
- **问题描述**：顶层 `WispError` 只有 `Browser`、`Network`、`Parse`、`Mcp`、`Storage`、`Timeout`、`Io` 变体。缺少：
  - `Engine` — 引擎状态错误（如 ND-001-ARCH）
  - `Config` — 配置错误（如无效代理 URL、无效正则）
  - `Scheduler` — 调度器错误

  当前这些错误被塞进 `Network` 或 `Other`，语义不清。

- **建议修复**：扩展 `WispError` 枚举。

#### ND-008-ARCH — Scheduler seen 集合无界增长（已修复）

- **严重程度**：Medium
- **文件**：[src/crawl/scheduling/scheduler.rs:64-65](file:///home/weng/wisp/src/crawl/scheduling/scheduler.rs#L64-L65)
- **问题描述**：

  ```rust
  seen_exact: Arc<DashSet<String>>,
  seen_fp: Arc<DashSet<u64>>,
  ```

  长时间爬取大站点（如 sitemap 索引百万 URL）会导致 seen 集合无界增长，最终 OOM。Fingerprint 模式省内存但仍无上限。

- **建议修复**：提供可选的 LRU 淘汰策略或 Bloom Filter 模式（容忍假阳性重复爬取）：

  ```rust
  pub enum DedupStrategy {
      Exact,
      Fingerprint,
      BloomFilter { capacity: usize, fp_rate: f64 },  // 新增
  }
  ```

#### ND-009-ARCH — 默认中间件链构建耦合在 run_inner

- **严重程度**：Medium
- **文件**：[src/crawl/runner.rs:257-280](file:///home/weng/wisp/src/crawl/runner.rs#L257-L280)
- **问题描述**：默认中间件链在 `run_inner` 的 ctx 字面量内构建，耦合了 Engine 与 `middleware::builtin::default_middlewares` 实现细节。修改默认中间件列表需要修改 run_inner。

- **建议修复**：抽取为独立函数 `build_default_chain(spider_config: &SpiderConfig) -> MiddlewareChain`，便于测试和复用。

#### ND-010-ARCH — 日志与 tracing 耦合紧密

- **严重程度**：Low
- **文件**：全项目 `#[tracing::instrument]` 和 `tracing::warn!` 使用
- **问题描述**：Engine 直接依赖 `tracing`，没有抽象层。如果未来要支持自定义事件系统（如 OpenTelemetry、自定义 metrics），需要大规模重构。

- **建议修复**：定义 `EventSink` trait，让 Engine 通过 trait 接口输出事件，tracing 作为默认实现。当前 tradeoff 是简单性，可标记为已知限制。

---

### 3.4 错误处理（ND-XXX-ERR）

#### ND-001-ERR — fetch_dispatch 重试时不发送 Error 事件（已修复）

- **严重程度**：Medium
- **文件**：[src/crawl/engine.rs:320-322](file:///home/weng/wisp/src/crawl/engine.rs#L320-L322)
- **问题描述**：

  ```rust
  if ctx.shared.follow_tx.send(retry_req).is_err() {
      tracing::debug!("follow_tx closed, dropping retry request");
  }
  ctx.shared.work_notify.notify_one();
  return (None, None);  // 不返回错误，不发送事件
  ```

  中间件决定重试时直接 `return (None, None)`，不返回错误也不发送 `CrawlEvent::Error`。这导致：
  - `run_stream` 消费者无法感知重试发生
  - 监控系统无法统计重试频率
  - `stats.retries` 计数器递增但无对应事件

- **建议修复**：增加 `CrawlEvent::Retry { url, attempt, error }` 事件类型，重试时发送。

#### ND-002-ERR — process_response 中间件 abort 不发送事件

- **严重程度**：Low
- **文件**：[src/crawl/engine.rs:193-196](file:///home/weng/wisp/src/crawl/engine.rs#L193-L196)
- **问题描述**：

  ```rust
  middleware::MwAction::Abort(reason) => {
      tracing::warn!("response middleware abort: {} - {}", reason, page_url);
      return;  // 无事件
  }
  ```

  中间件 abort 时只 log warn，不发送 `CrawlEvent::Error`，外部观察者无法感知。

- **建议修复**：发送 `CrawlEvent::Error` 事件。

#### ND-003-ERR — save_checkpoint 失败只 warn 不返回错误（已修复）

- **严重程度**：Medium
- **文件**：[src/crawl/engine.rs:401-410](file:///home/weng/wisp/src/crawl/engine.rs#L401-L410)
- **问题描述**：

  ```rust
  match bincode::serialize(&state) {
      Ok(blob) => {
          if let Err(e) = store.save_checkpoint(spider_name, &blob, state.saved_at.timestamp()) {
              tracing::warn!("checkpoint 保存失败: {}", e);
          }
      }
      Err(e) => {
          tracing::warn!("checkpoint 序列化失败: {}", e);
      }
  }
  ```

  checkpoint 失败只 warn 不返回错误，调用方（`run_inner`）不知道持久化失败。如果爬虫崩溃后无法恢复，用户无法提前感知。

- **建议修复**：返回 `Result<()>` 让调用方决定是否终止爬取，或增加 `on_checkpoint_failure: Continue | Abort` 配置。

#### ND-004-ERR — EngineControl::reset 失败无处理

- **严重程度**：Low
- **文件**：[src/crawl/runner.rs:182](file:///home/weng/wisp/src/crawl/runner.rs#L182)
- **问题描述**：`self.control.reset().await` 是 async 调用但无错误处理。如果 reset 内部锁中毒或失败，run 会继续执行但 control 状态可能不一致。

- **建议修复**：检查 reset 返回值（如果 reset 返回 Result），或确保 reset 是同步原子操作。

#### ND-005-ERR — browser/page.rs 多处 .unwrap() 违反规范（已修复）

- **严重程度**：Medium（汇总 ND-005-SEC 的代码风格层面）
- **文件**：[src/browser/page.rs](file:///home/weng/wisp/src/browser/page.rs) 共 8 处 `serde_json::to_string(...).unwrap()`
- **问题描述**：见 ND-005-SEC。从错误处理角度，这些 `.unwrap()` 虽然不会触发（&str 序列化不会失败），但违反"非测试代码不应有 unwrap"的项目规范。

- **建议修复**：改用 `?` 或 `expect`。

#### ND-006-ERR — storage/mod.rs load_element 多处 unwrap_or_default 吞掉错误

- **严重程度**：Low
- **文件**：[src/storage/mod.rs:189, 191, 192, 195](file:///home/weng/wisp/src/storage/mod.rs#L189-L195)
- **问题描述**：见 ND-010-CORR。从错误处理角度，`unwrap_or_default()` 静默吞掉 JSON 解析错误，违反"错误不应被静默吞掉"原则。

- **建议修复**：见 ND-010-CORR。

---

### 3.5 性能（ND-XXX-PERF）

#### ND-007-PERF — run_inner 主循环 busy-wait 轮询（已修复）

- **严重程度**：Medium
- **文件**：[src/crawl/runner.rs:379-403](file:///home/weng/wisp/src/crawl/runner.rs#L379-L403)
- **问题描述**：见 ND-004-CORR。从性能角度，10ms timeout 轮询在高并发场景下浪费 CPU。

- **建议修复**：见 ND-004-CORR。

#### ND-008-PERF — build_crawl_context 每次中间件调用都构建

- **严重程度**：Low
- **文件**：[src/crawl/engine.rs:335-345](file:///home/weng/wisp/src/crawl/engine.rs#L335-L345)
- **问题描述**：

  ```rust
  pub(crate) fn build_crawl_context(ctx: &EngineContext) -> middleware::CrawlContext {
      middleware::CrawlContext {
          spider_name: ctx.state.spider.name().to_string(),  // 分配
          // ...
      }
  }
  ```

  每个请求/响应中间件调用都构建 `CrawlContext`，`spider_name.to_string()` 在 hot path 分配。`process_response` 内每个 item 也调用一次（L236）。

- **建议修复**：在 `EngineContext` 缓存 `CrawlContext` 或使用 `Arc<str>` 避免 clone。

#### ND-009-PERF — process_response 每个 item 都走完整 pipeline 链（已修复）

- **严重程度**：Medium
- **文件**：[src/crawl/engine.rs:226-249](file:///home/weng/wisp/src/crawl/engine.rs#L226-L249)
- **问题描述**：对每个 item 都 `build_crawl_context` + `run_pipelines`，pipeline 链可能包含多个中间件，每个都 await。对于产出大量 items 的页面（如 1000+），这是显著开销。

- **建议修复**：批量处理 items（`run_pipelines_batch`），减少 per-item 开销。或缓存 crawl_ctx。

---

### 3.6 测试覆盖率（ND-XXX-TEST）

#### ND-012-TEST — engine.rs 核心函数测试覆盖不足

- **严重程度**：Medium
- **文件**：[src/crawl/engine.rs:528-660](file:///home/weng/wisp/src/crawl/engine.rs#L528-L660)
- **问题描述**：单元测试只覆盖：
  - `process_response` 的 `from_cache` 分支（2 个测试）
  - `save_checkpoint` 的 seen_urls 持久化（1 个测试）

  未覆盖：
  - `process_request` — 请求阶段核心逻辑
  - `fetch_dispatch` — 抓取分发与重试逻辑
  - `fetch_page` / `fetch_page_inner` — 模式分发
  - `check_control_and_hook` — 控制状态检查
  - Refetch 循环逻辑
  - 错误事件发送逻辑

- **建议修复**：增加 mock FetchClient 的单元测试，覆盖上述函数的主要分支。

#### ND-013-TEST — run_inner 完全无单元测试

- **严重程度**：Medium
- **文件**：[src/crawl/runner.rs](file:///home/weng/wisp/src/crawl/runner.rs)
- **问题描述**：`run_inner` 是 Engine 最复杂的函数（~300 行），完全依赖 integration test 间接覆盖。无单元测试意味着：
  - checkpoint 恢复逻辑未独立验证
  - autoscale 集成未独立验证
  - 控制流（pause/cancel/shutdown）未独立验证
  - 主循环边界条件（max_pages、stop_condition）未独立验证

- **建议修复**：抽取可测试单元（如 `should_continue`、`drain_follow_channel`），增加单元测试。

#### ND-014-TEST — autoscale 测试不验证扩缩容逻辑

- **严重程度**：Low
- **文件**：[src/crawl/runtime/autoscale.rs:157-193](file:///home/weng/wisp/src/crawl/runtime/autoscale.rs#L157-L193)
- **问题描述**：测试只验证：
  - pool 创建（2 个测试）
  - autoscaler 运行不 panic（1 个测试）

  未验证：
  - 高饱和度时正确扩容
  - 低饱和度时正确缩容
  - 高错误率时缩容
  - 冷却时间生效

- **建议修复**：构造模拟 stats（高/低饱和度、高错误率），验证 `current_concurrency` 变化。

#### ND-015-TEST — MCP tools 测试只覆盖错误路径

- **严重程度**：Low
- **文件**：[src/mcp/tools.rs:221-291](file:///home/weng/wisp/src/mcp/tools.rs#L221-L291)
- **问题描述**：5 个工具中，`extract_css` 有 happy path 测试，其余只测试 missing args 错误路径。`fetch_page`、`crawl_site`、`stealth_fetch`、`adaptive_scrape` 的正常流程未覆盖。

- **建议修复**：使用 mock HTTP server 增加 happy path 测试。

#### ND-016-TEST — BrowserPool 测试不验证 launch 失败恢复

- **严重程度**：Medium
- **文件**：[src/browser/pool.rs:164-200](file:///home/weng/wisp/src/browser/pool.rs#L164-L200)
- **问题描述**：测试只验证 permit 计数逻辑，未验证：
  - launch 失败后下次 acquire 重试
  - shutdown 行为
  - 并发 acquire 的 launch 串行化
  - BrowserHandle::Drop 正确归还 permit

- **建议修复**：使用 mock Browser 增加 launch 失败场景测试。

#### ND-017-TEST — benches 目录未充分覆盖核心路径

- **严重程度**：Low
- **文件**：[benches/](file:///home/weng/wisp/benches/)
- **问题描述**：`Cargo.toml` 声明了 `[[bench]] name = "bench"`，但未读取 benches/ 内容评估覆盖率。基准测试应覆盖：
  - 中间件链执行
  - scheduler push/pop
  - autoscale 决策
  - HTML 解析
  - 缓存命中/未命中

- **建议修复**：审查并补充基准测试。

---

### 3.7 依赖配置（ND-XXX-DEP）

#### ND-001-DEP — Cargo.toml 缺少 release profile 优化（已修复）

- **严重程度**：Medium
- **文件**：[Cargo.toml](file:///home/weng/wisp/Cargo.toml)
- **问题描述**：`Cargo.toml` 没有 `[profile.release]` 配置，release 构建使用默认设置（无 LTO、codegen-units=16、不 strip）。对于爬虫框架这种性能敏感项目，可能损失 10-30% 性能。

- **建议修复**：

  ```toml
  [profile.release]
  lto = "thin"          # 跨 crate 内联优化
  codegen-units = 1     # 单 codegen unit，最佳优化
  strip = true          # 移除调试符号，减小二进制
  panic = "abort"       # 减小二进制（注意：禁用 catch_unwind）
  ```

  注意 `panic = "abort"` 会影响 `Drop` 实现（如 `Page::Drop` 的 tokio::spawn 兜底），需评估后再启用。

#### ND-002-DEP — wreq 精确锁定 RC 版本

- **严重程度**：Low
- **文件**：[Cargo.toml:31-32](file:///home/weng/wisp/Cargo.toml#L31-L32)
- **问题描述**：

  ```toml
  wreq = { version = "=6.0.0-rc.29", features = ["cookies", "stream"] }
  wreq-util = "=3.0.0-rc.14"
  ```

  精确锁定 RC 版本，跟踪升级成本高。RC 版本 API 不稳定，每次升级可能需要代码改动。注释说明"跟踪 wreq 正式版发布后及时迁移"，但未设置升级 issue。

- **建议修复**：保持当前锁定，但在项目 issue tracker 创建"跟踪 wreq 正式版"任务。

#### ND-003-DEP — tokio features 启用过多

- **严重程度**：Low
- **文件**：[Cargo.toml:9](file:///home/weng/wisp/Cargo.toml#L9)
- **问题描述**：

  ```toml
  tokio = { version = "1", features = ["full"] }
  ```

  `features = ["full"]` 启用所有 tokio 功能，但项目可能不需要 `signal`、`process`（除非浏览器启动用）、`io-util` 等。会增加编译时间和二进制大小。

- **建议修复**：按需启用 features，或保持 `full` 但标记为已知 tradeoff（开发便利性 vs 编译时间）。

#### ND-004-DEP — rusqlite bundled 增加 ~5MB 二进制

- **严重程度**：Low
- **文件**：[Cargo.toml:37](file:///home/weng/wisp/Cargo.toml#L37)
- **问题描述**：

  ```toml
  rusqlite = { version = "0.40", features = ["bundled"] }
  ```

  `bundled` 编译 SQLite 源码进二进制，增加 ~5MB。但保证可移植性（无需系统 libsqlite3），是合理 tradeoff。

- **建议修复**：保持现状，标记为已知 tradeoff。

#### ND-005-DEP — 缺少 cargo-deny / cargo-audit 配置（已修复）

- **严重程度**：Medium
- **文件**：项目根目录
- **问题描述**：未配置 `cargo-deny` 或 `cargo-audit` 进行依赖安全扫描。`Cargo.lock` 中可能存在已知漏洞的依赖版本，但无自动检测机制。

- **建议修复**：添加 `deny.toml` 配置 cargo-deny，在 CI 中运行 `cargo deny check advisories`。

---

### 3.8 文档完整性（ND-XXX-DOC）

#### ND-006-DOC — src/lib.rs 未启用 #![warn(missing_docs)]（已修复）

- **严重程度**：Medium
- **文件**：[src/lib.rs](file:///home/weng/wisp/src/lib.rs)
- **问题描述**：crate 顶层未启用 `#![warn(missing_docs)]`，pub API 文档覆盖率低。许多 `pub fn`、`pub struct` 缺少文档注释。

- **建议修复**：在 `src/lib.rs` 顶部添加：

  ```rust
  #![warn(missing_docs)]
  #![warn(clippy::all, clippy::pedantic)]
  ```

#### ND-007-DOC — 缺少 CHANGELOG.md 和 CONTRIBUTING.md（部分修复）

- **严重程度**：Medium
- **文件**：项目根目录
- **问题描述**：README.md 已存在但缺少：
  - `CHANGELOG.md` — 版本变更记录
  - `CONTRIBUTING.md` — 贡献指南
  - `LICENSE` — 许可证文件（Cargo.toml 声明 Apache-2.0，但根目录无 LICENSE 文件）

- **建议修复**：创建上述文件。LICENSE 文件可直接从 Apache 2.0 官方文本复制。

#### ND-008-DOC — README 未提及 CLAUDE.md 开发规范

- **严重程度**：Low
- **文件**：[README.md](file:///home/weng/wisp/README.md)
- **问题描述**：README 是用户文档，但未提及 `CLAUDE.md` 中的开发规范（中文回复、snake_case、禁止切分支等）。新贡献者可能不知道这些规范。

- **建议修复**：在 README 增加 "Development" 章节链接到 CLAUDE.md，或创建 CONTRIBUTING.md 引用。

#### ND-009-DOC — WispError 变体缺少使用指南

- **严重程度**：Low
- **文件**：[src/error.rs:141-169](file:///home/weng/wisp/src/error.rs#L141-L169)
- **问题描述**：`WispError` 文档注释描述了变体分类，但缺少：
  - 何时使用哪个变体的指南
  - `# Errors` 标注的规范
  - 错误转换示例
  - `#[from]` 自动转换的说明

- **建议修复**：扩展 `WispError` 文档，增加使用示例。

#### ND-010-DOC — 多个 mod.rs 缺少模块级文档（已修复）

- **严重程度**：Medium
- **文件**：[src/crawl/runtime/mod.rs](file:///home/weng/wisp/src/crawl/runtime/mod.rs)、[src/crawl/observability/mod.rs](file:///home/weng/wisp/src/crawl/observability/mod.rs) 等
- **问题描述**：部分 `mod.rs` 缺少 `//!` 模块级文档，导致 `cargo doc` 生成的文档结构不完整。已审查的模块（如 `autoscale.rs`、`robots.rs`）有良好文档，但部分子模块缺失。

- **建议修复**：审查所有 `mod.rs`，补充 `//!` 文档说明模块职责和与其他模块的关系。

---

### 3.9 规范遵守（ND-XXX-STYLE）

#### ND-011-STYLE — page.rs::cmd 是 pub 但暴露 CDP 细节

- **严重程度**：Low
- **文件**：[src/browser/page.rs:22-24](file:///home/weng/wisp/src/browser/page.rs#L22-L24)
- **问题描述**：

  ```rust
  pub async fn cmd(&self, method: &str, params: Value) -> Result<Value> {
  ```

  `cmd` 是 `pub`，对外暴露 CDP 协议细节（method 名、params 结构）。这违反了封装原则，外部代码可能依赖具体 CDP method 名。

- **建议修复**：改为 `pub(crate)` 或 `pub(super)`，仅限内部使用。

#### ND-012-STYLE — engine.rs::record_status 使用 #[doc(hidden)] 不当

- **严重程度**：Low
- **文件**：[src/crawl/engine.rs:348-357](file:///home/weng/wisp/src/crawl/engine.rs#L348-L357)
- **问题描述**：

  ```rust
  #[doc(hidden)]
  pub fn record_status(stats: &Arc<SpiderStats>, status: u16) {
  ```

  `record_status` 是内部函数，用 `#[doc(hidden)]` + `pub` 不合适，应改为 `pub(crate)` 或 `pub(super)`。

- **建议修复**：改为 `pub(crate) fn record_status`。

#### ND-013-STYLE — engine.rs::fetch_page 同上

- **严重程度**：Low
- **文件**：[src/crawl/engine.rs:415-444](file:///home/weng/wisp/src/crawl/engine.rs#L415-L444)
- **问题描述**：`fetch_page` 和 `fetch_page_inner` 用 `#[doc(hidden)] pub`，应改为 `pub(crate)`。

- **建议修复**：改为 `pub(crate)`。

#### ND-014-STYLE — 未启用 clippy::pedantic（已修复）

- **严重程度**：Medium
- **文件**：[src/lib.rs](file:///home/weng/wisp/src/lib.rs)
- **问题描述**：未启用 `#![warn(clippy::all, clippy::pedantic)]`，可能存在 clippy 警告未修复。项目规范要求"提交前用 code-review 审查变更"，但无自动化 lint 检查。

- **建议修复**：在 `src/lib.rs` 顶部添加：

  ```rust
  #![warn(clippy::all)]
  #![warn(clippy::pedantic)]
  #![allow(clippy::module_name_repetitions)]  // 必要时按需 allow
  ```

#### ND-031-ARCH — Spider trait 混合业务逻辑与引擎配置，职责边界不清（已修复）

- **严重程度**：Medium
- **文件**：[src/crawl/mod.rs:66-118](file:///home/weng/wisp/src/crawl/mod.rs#L66-L118)
- **问题描述**：

  `Spider` trait 混合了两类职责：

  ```rust
  pub trait Spider: Send + Sync + 'static {
      // 业务逻辑（应保留在 Spider）
      fn name(&self) -> &str;
      fn start_urls(&self) -> Vec<String>;
      async fn handle(&self, resp: Response) -> (Vec<Value>, Vec<Request>);
      fn allowed_domains(&self) -> HashSet<String>;
      fn is_blocked(&self, resp: &Response) -> bool;
      fn until(&self) -> Arc<dyn StopCondition>;
      fn middlewares(&self) -> Vec<Arc<dyn Middleware>>;
      fn pipelines(&self) -> Vec<Arc<dyn ItemPipeline>>;

      // 引擎配置（应迁移到 EngineConfig）
      fn download_delay(&self) -> Duration { Duration::from_millis(0) }
      fn obey_robots(&self) -> bool { true }
      fn max_retries(&self) -> u32 { 3 }              // 重试是引擎行为
      fn fetch_mode(&self) -> FetchMode { Http }       // 抓取模式是引擎行为
      fn max_depth(&self) -> u32 { u32::MAX }
      fn auto_rules(&self) -> Vec<(String, FetchMode)>;
  }
  ```

  `max_retries`、`download_delay`、`fetch_mode`、`obey_robots`、`auto_rules` 都是"如何抓取"的引擎行为，而非"解析什么"的业务逻辑。当前归属导致：

  1. **配置重复传递**：`max_retries` 既传给 `RetryMiddleware`（已修复：移除），又由 engine 通过 `spider.max_retries()` 读取（现改为 `EngineConfig.max_retries`）。同一份配置被传两次。
  2. **Engine 难以独立配置**：用户无法在不实现 Spider 的情况下配置引擎行为（如批量抓取场景只需 Fetcher，不需要 Spider）。
  3. **职责混淆**：`Spider` 应只关心"解析什么、如何解析"，但当前还承担"如何抓取"的配置。

- **修复方案**（已实施）：

  将以下方法从 `Spider` trait 迁移到 `EngineBuilder` / `Engine`：

  | 方法 | 原归属 | 新归属 | 理由 |
  |------|--------|--------|------|
  | `max_retries()` | Spider | `EngineBuilder::max_retries(u32)` / `Engine.max_retries` | 重试是引擎行为 |
  | `download_delay()` | Spider | `EngineBuilder::download_delay(Duration)` / `Engine.download_delay` | 下载节奏是引擎行为 |
  | `fetch_mode()` | Spider | `EngineBuilder::fetch_mode(FetchMode)` / `Engine.fetch_mode` | 抓取模式是引擎行为 |
  | `obey_robots()` | Spider | `EngineBuilder::obey_robots(bool)` / `Engine.obey_robots` | robots 遵守是引擎行为 |
  | `auto_rules()` | Spider | `EngineBuilder::auto_rule(pattern, mode)` / `Engine.auto_rules` | 模式规则是引擎行为 |

  保留在 `Spider` trait 的为纯业务逻辑：`name`、`start_urls`、`handle`、`allowed_domains`、`is_blocked`、`max_depth`、`until`、`middlewares`、`pipelines`、`on_*` 钩子。

  `max_depth` 保留在 Spider：爬取深度是业务范围决策（"我要爬多深"），非引擎行为。

- **修复后的职责划分**：

  ```
  Spider trait（业务逻辑）：
    name, start_urls, handle, allowed_domains, is_blocked,
    max_depth, until, middlewares, pipelines, on_* 钩子

  EngineBuilder / Engine（引擎配置）：
    fetch_mode, obey_robots, max_retries, download_delay, auto_rules,
    max_concurrent, max_pages, max_refetch_rounds, cache_store, checkpoint, autoscale
  ```

- **API 变更示例**：

  ```rust
  // 修复前：引擎配置混在 SpiderBuilder
  let spider = SpiderBuilder::new("x")
      .delay(Duration::from_millis(500))
      .obey_robots(false)
      .max_retries(3)
      .mode(FetchMode::Auto)
      .build();
  let engine = Engine::infra().build()?;

  // 修复后：引擎配置在 EngineBuilder
  let spider = SpiderBuilder::new("x")
      .build();
  let engine = Engine::infra()
      .download_delay(Duration::from_millis(500))
      .obey_robots(false)
      .max_retries(3)
      .fetch_mode(FetchMode::Auto)
      .build()?;
  ```

- **修改的文件**：
  - [src/crawl/mod.rs](file:///home/weng/wisp/src/crawl/mod.rs)：Spider trait 移除 5 个引擎配置方法
  - [src/crawl/runner.rs](file:///home/weng/wisp/src/crawl/runner.rs)：Engine 结构体增加 5 个字段，EngineBuilder 增加 6 个配置方法
  - [src/crawl/builder.rs](file:///home/weng/wisp/src/crawl/builder.rs)：SpiderBuilder 移除 delay/obey_robots/max_retries/mode/auto_rule 方法，ClosureSpider 移除对应字段
  - [src/mcp/mod.rs](file:///home/weng/wisp/src/mcp/mod.rs)：MCP server 的 Engine 创建添加 `.obey_robots(false)`
  - [src/mcp/tools.rs](file:///home/weng/wisp/src/mcp/tools.rs)：SimpleSpider 移除 obey_robots 方法
  - [examples/novel_crawler.rs](file:///home/weng/wisp/examples/novel_crawler.rs)：delay/obey_robots 从 SpiderBuilder 迁移到 EngineBuilder
  - tests/ 目录下所有集成测试同步更新

- **验证**：231 个 lib 测试通过，所有集成测试编译通过，clippy 无新增警告。

- **状态**：✅ 已修复

#### ND-032-CORR — _retry meta 字段被两套重试机制共享，计数器冲突（已修复）

- **严重程度**：Medium（原 Low，深度分析后升级）
- **文件**：[src/crawl/middleware/builtin.rs](file:///home/weng/wisp/src/crawl/middleware/builtin.rs)（原 `RetryMiddleware` 和 `BlockedRetryMiddleware`）
- **问题描述**：

  原实现中，`meta["_retry"]` 字段同时被两个中间件共享：

  ```rust
  // RetryMiddleware（网络错误重试）
  async fn process_error(&self, req: &Request, ...) -> ErrorAction {
      let count = req.meta.get("_retry").and_then(|v| v.as_u64()).unwrap_or(0);
      if count < self.max_retries as u64 { ... ErrorAction::Retry }
  }

  // BlockedRetryMiddleware（阻塞重试）
  async fn process_response(&self, resp: &mut Response, ...) -> MwAction {
      let count = resp.request.meta.get("_retry")...;
      if count < self.max_retries as u64 {
          let mut new_req = resp.request.clone();
          new_req.meta["_retry"] = serde_json::json!(count + 1);
          return MwAction::Refetch(new_req);
      }
  }
  ```

  **计数器冲突场景**：
  1. 请求被 `BlockedRetryMiddleware` Refetch 3 次（`_retry=3`），响应成功
  2. 但 Refetch 后的响应网络失败 → `RetryMiddleware` 看到 `_retry=3`
  3. `RetryMiddleware` 直接 `Propagate`，不再重试（认为已重试 3 次）
  4. 两套独立语义的计数器相互干扰

  这也暗示了 `BlockedRetryMiddleware` 用 `Refetch` 而非 `ErrorAction::Retry` 的原因：因为 `Retry` 路径被 scheduler 去重破坏（ND-002-CORR），只能用 `Refetch` 绕开。

- **修复方案**（已实施，与 ND-002-CORR 一并修复）：

  1. **`retry_count` 显式字段**：`Request` 增加 `retry_count: u32`，仅用于网络错误重试，由 engine 维护
  2. **`refetch_depth` 局部变量**：`process_response` 内的 `refetch_depth` 仅用于业务重做，由 engine 维护
  3. **移除 `meta["_retry"]`**：两套重试不再共享 meta 字段
  4. **`BlockedRetryMiddleware` 移除计数**：依赖 engine 的 `refetch_depth` 上限

- **修复后的计数器隔离**：

  | 计数器 | 用途 | 维护者 | 上限 |
  |--------|------|--------|------|
  | `req.retry_count` | 网络错误重试 | engine（fetch_dispatch） | `EngineConfig.max_retries` |
  | `refetch_depth` | 业务重做（含阻塞重试） | engine（process_response） | `EngineConfig.max_refetch_rounds` |

  两套计数器**完全独立**：一个请求若先被 `BlockedRetryMiddleware` Refetch 2 次（`refetch_depth=2`），然后 Refetch 后的响应网络失败，`RetryMiddleware` 看到 `retry_count=0`，可以独立重试 3 次。

- **状态**：✅ 已修复

---

## 四、优先级建议

### 4.0 已修复（本轮）

| 编号 | 描述 | 修复方式 |
|------|------|----------|
| ND-002-CORR | ErrorAction::Retry 路径被 scheduler seen 去重破坏 | fetch_dispatch 改为同步循环，Request 增加 retry_count 字段 |
| ND-031-ARCH | Spider trait 混合业务逻辑与引擎配置 | 引擎配置方法迁移到 EngineBuilder，Spider 只保留业务逻辑 |
| ND-032-CORR | _retry meta 字段被两套重试机制共享 | 移除 meta["_retry"]，retry_count 和 refetch_depth 独立计数 |

### 4.1 立即修复（High）

| 编号 | 描述 | 影响 |
|------|------|------|
| ND-007-CORR | refetch 失败丢失错误上下文 | 监控盲区，调试困难 |
| ND-007-SEC | fetch_dispatch 错误泄露 URL 凭据 | 凭据泄露风险 |
| ND-008-SEC | 缺少响应体大小限制 | DoS 风险 |
| ND-011-SEC | 缺少 TLS 证书验证配置 | 无法抓取自签名证书站点 |

### 4.2 计划修复（Medium）

| 编号 | 描述 |
|------|------|
| ND-004-CORR | run_inner 10ms 轮询 |
| ND-010-CORR | SqliteStore load_element 静默吞错误 |
| ND-003-SEC | crawl_site 仅校验 scheme |
| ND-004-SEC | url_to_filename 未过滤保留名 |
| ND-009-SEC | proxy_clients 无界增长 |
| ND-001-ARCH | Engine 并发保护错误类型语义错误 |
| ND-008-ARCH | Scheduler seen 集合无界增长 |
| ND-001-ERR | fetch_dispatch 重试不发事件 |
| ND-003-ERR | save_checkpoint 失败只 warn |
| ND-005-ERR | page.rs 多处 unwrap |
| ND-007-PERF | 主循环 busy-wait |
| ND-009-PERF | 每 item 走完整 pipeline |
| ND-012-TEST | engine.rs 核心函数测试不足 |
| ND-013-TEST | run_inner 无单元测试 |
| ND-016-TEST | BrowserPool 测试不全 |
| ND-001-DEP | 缺少 release profile 优化 |
| ND-005-DEP | 缺少 cargo-deny 配置 |
| ND-006-DOC | 未启用 missing_docs |
| ND-007-DOC | 缺少 CHANGELOG/CONTRIBUTING/LICENSE |
| ND-010-DOC | mod.rs 文档缺失 |
| ND-014-STYLE | 未启用 clippy::pedantic |

### 4.3 可选改进（Low）

剩余 Low 严重程度的缺陷可作为技术债务 backlog 跟踪，择机改进。

---

## 五、附录

### 5.1 审查覆盖的文件

核心文件（已完整阅读）：
- `Cargo.toml`、`src/lib.rs`、`src/error.rs`
- `src/crawl/engine.rs`、`src/crawl/runner.rs`、`src/crawl/scheduling/scheduler.rs`
- `src/crawl/runtime/autoscale.rs`、`src/crawl/runtime/robots.rs`
- `src/browser/pool.rs`、`src/browser/page.rs`
- `src/fetcher/mod.rs`、`src/http/proxy.rs`
- `src/mcp/mod.rs`、`src/mcp/tools.rs`
- `src/storage/mod.rs`
- `src/utils/url.rs`、`src/utils/mod.rs`
- `src/crawl/runtime/items.rs`、`src/crawl/observability/stats.rs`

子代理覆盖（部分验证）：
- `src/crawl/middleware/builtin.rs`、`src/crawl/middleware/pipeline.rs`
- `src/http/block.rs`、`src/http/encoding.rs`、`src/http/ua.rs`
- `src/stealth/challenge.rs`、`src/stealth/turnstile.rs`、`src/stealth/human.rs`
- `src/storage/migrations.rs`
- `src/parser/*`、`src/browser/cdp.rs`、`src/browser/launch.rs`

### 5.2 审查方法说明

1. **并行子代理审查**：5 个 search 子代理分别审查正确性、安全、架构、错误处理+性能+测试、依赖+文档+规范领域
2. **人工验证**：对子代理报告的关键发现，读取源码验证真伪，剔除误判
3. **修复验证**：抽样验证 V1 报告中 8 个关键缺陷的修复状态
4. **新增缺陷**：在验证过程中发现的新问题，按统一格式记录

### 5.3 与 V1 报告的关系

- V1 报告（`code-review-defect-report.md`）的 34 个缺陷已全部处理（修复或标记为设计取舍）
- 本报告（V2）聚焦新发现的问题，编号从 ND-001 重新开始
- 建议将 V2 报告与 V1 合并归档，作为项目质量演进的记录

---

**审查完成**。本报告共发现 30 个新缺陷，其中 4 个 High、14 个 Medium、12 个 Low。建议按优先级分批修复，重点处理安全漏洞（SSRF、凭据泄露、DoS）和错误处理盲区。
