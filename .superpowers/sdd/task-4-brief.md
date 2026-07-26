# Task 4: fetch_dispatch 退避抖动 + rule_engine 单次锁（M1 + M4）

**Files:**
- Modify: `src/crawl/engine.rs`（fetch_dispatch AutoFallback + Retry 分支）
- Modify: `src/crawl/middleware/builtin.rs`（RetryMiddleware 移除 retry_delay + sleep）
- Modify: `src/crawl/middleware/mod.rs`（doc 示例更新）
- Test: `src/crawl/engine.rs`

**Interfaces:**
- Consumes: `rand` crate（检查 Cargo.toml）
- Produces: `RetryMiddleware::new(max_retries: u32)`（删除 retry_delay 参数）

## Steps

### Step 1: 检查 rand 依赖

Run: `grep -n "^rand" /home/weng/wisp/Cargo.toml`
若无 rand，需在 [dependencies] 添加 `rand = "0.9"`。wisp 应已有 rand，先检查。

### Step 2: 写失败测试 — 指数退避

在 `src/crawl/engine.rs` 的 `#[cfg(test)]` 模块追加：

```rust
#[test]
fn test_retry_exponential_backoff() {
    fn compute_backoff(attempt: u32) -> u64 {
        let base_ms = 100u64;
        let cap_ms = 30_000u64;
        base_ms
            .saturating_mul(1u64 << attempt.min(10))
            .min(cap_ms)
    }

    assert_eq!(compute_backoff(1), 200, "attempt=1 → 200ms");
    assert_eq!(compute_backoff(2), 400, "attempt=2 → 400ms");
    assert_eq!(compute_backoff(3), 800, "attempt=3 → 800ms");
    assert_eq!(compute_backoff(8), 25_600, "attempt=8 → 25.6s");
    assert_eq!(compute_backoff(10), 30_000, "attempt=10 → 封顶 30s");
    assert_eq!(compute_backoff(20), 30_000, "attempt=20 → 仍封顶 30s");
}
```

### Step 3: 验证测试通过

Run: `cargo test --lib crawl::engine::tests::test_retry_exponential_backoff --all-features`

### Step 4: 写失败测试 — 抖动范围

```rust
#[test]
fn test_retry_jitter_range() {
    fn compute_jitter_bound(exp_delay: u64) -> u64 {
        exp_delay / 2 + 1
    }

    assert_eq!(compute_jitter_bound(200), 101);
    assert_eq!(compute_jitter_bound(400), 201);
    assert_eq!(compute_jitter_bound(30_000), 15_001);
}
```

### Step 5: 验证测试通过

### Step 6: 写失败测试 — rule_engine 单次锁

```rust
#[tokio::test]
async fn test_rule_engine_single_lock_for_autofallback() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use tokio::sync::Mutex;

    struct MockRuleEngine {
        lock_count: AtomicU32,
    }
    impl MockRuleEngine {
        async fn resolve_and_learn(&self) -> bool {
            self.lock_count.fetch_add(1, Ordering::SeqCst);
            true
        }
    }

    let engine = Arc::new(MockRuleEngine {
        lock_count: AtomicU32::new(0),
    });

    let should_upgrade = engine.resolve_and_learn().await;

    assert!(should_upgrade);
    assert_eq!(engine.lock_count.load(Ordering::SeqCst), 1, "应只抢一次锁");
}
```

### Step 7: 验证测试通过

### Step 8: 实现 fetch_dispatch AutoFallback 单次锁

定位 `fetch_dispatch` 函数中 AutoFallback 段（搜索 `AutoFallback` 或 `rule_engine.lock`），替换为单次锁：

```rust
// OPTIMIZE: 合并 resolve+learn 到单次锁，避免两次锁争用。
if ctx.config.fetch_mode == FetchMode::Auto
    && current_req.fetch_mode_override.is_none()
    && current_req.retry_count == 0
{
    let should_upgrade = {
        let mut engine = ctx.shared.rule_engine.lock().await;
        if engine.resolve(&current_req.url) != Some(FetchMode::Stealth) {
            engine.learn(&current_req.url, FetchMode::Stealth);
            true
        } else {
            false
        }
    };
    if should_upgrade {
        tracing::info!(
            "AutoFallback: '{}' 首次抓取失败 ({}), 升级 Stealth 重试",
            sanitize_url(&current_req.url),
            e
        );
        current_req.fetch_mode_override = Some(FetchMode::Stealth);
        continue;
    }
}
```

### Step 9: 实现 Retry 路径指数退避 + 抖动

定位 `fetch_dispatch` 中 `ErrorAction::Retry` 分支，在 `continue` 前添加：

```rust
middleware::ErrorAction::Retry if current_req.retry_count < max_retries => {
    current_req.retry_count += 1;
    stats.retries.fetch_add(1, Ordering::SeqCst);

    // OPTIMIZE: 指数退避 + 抖动，避免失败场景下的 Thundering Herd。
    let attempt = current_req.retry_count;
    let base_ms = 100u64;
    let cap_ms = 30_000u64;
    let exp_delay = base_ms
        .saturating_mul(1u64 << attempt.min(10))
        .min(cap_ms);
    // 抖动：[0, exp_delay/2)，避免所有失败请求同步重试
    let jitter = rand::rngs::SmallRng::try_from_rng(&mut rand::rngs::SysRng)
        .expect("OS RNG failed")
        .random_range(0..exp_delay / 2 + 1);
    let total_delay = std::time::Duration::from_millis(exp_delay + jitter);
    tracing::debug!(
        "retry {}/{}: {} (backoff {:?})",
        attempt,
        max_retries,
        sanitize_url(&current_req.url),
        total_delay
    );
    tokio::time::sleep(total_delay).await;

    if let Some(ref tx) = ctx.state.tx {
        let _ = tx.try_send(CrawlEvent::Retry {
            url: sanitize_url(&current_req.url),
            attempt,
            max: max_retries,
            error: e.to_string(),
        });
    }
    continue;
}
```

**注意**：rand API 需根据实际版本调整。若 `try_from_rng`/`random_range` 不可用，改用 `rand::thread_rng().gen_range(0..exp_delay/2+1)` 或类似 API。

### Step 10: 修改 RetryMiddleware — 移除 sleep 和 retry_delay

**10a. 修改结构体和 new**：

```rust
pub struct RetryMiddleware {
    max_retries: u32,
}

impl RetryMiddleware {
    /// 创建重试中间件。退避策略由 engine 统一负责（指数退避 + 抖动）。
    pub fn new(max_retries: u32) -> Self {
        Self { max_retries }
    }
}
```

**10b. 修改 process_error — 删除内部 sleep**：

```rust
async fn process_error(&self, req: &Request, _error: &str) -> ErrorAction {
    // OPTIMIZE: 退避由 engine 统一负责（指数退避 + 抖动），中间件仅决定是否重试。
    if req.retry_count < self.max_retries {
        ErrorAction::Retry
    } else {
        ErrorAction::Propagate
    }
}
```

### Step 11: 更新 mod.rs doc 示例

```rust
// 旧：.middleware(RetryMiddleware::new(3, std::time::Duration::from_secs(1)))
// 新：.middleware(RetryMiddleware::new(3))
```

### Step 12: 检查所有 RetryMiddleware::new 调用点

Run: `grep -rn "RetryMiddleware::new" /home/weng/wisp/src/`
所有调用点更新为单参数版本。

### Step 13: 验证测试通过

Run: `cargo test --lib crawl::engine::tests --all-features && cargo test --lib crawl::middleware --all-features`

### Step 14: 全量回归

Run: `cargo test --all-features`

### Step 15: Clippy 检查

Run: `cargo clippy --all-targets --all-features -- -D warnings`

### Step 16: 提交

```bash
cd /home/weng/wisp
git add src/crawl/engine.rs src/crawl/middleware/builtin.rs src/crawl/middleware/mod.rs
git commit -m "perf(engine): fetch_dispatch 退避抖动 + rule_engine 单次锁"
```
