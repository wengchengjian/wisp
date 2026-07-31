# Tracing 性能埋点设计

## 背景与动机

`cargo bench` 的 `engine_concurrent_fetch` benchmark 是黑盒端到端计时，无法拆解 Engine 内部各阶段（HTTP fetch / 中间件链 / 调度 / parse）各占多少耗时。需要一种方式在 benchmark 里看到各阶段耗时的百分比拆解，定位瓶颈。

## 业界调研结论

- **Scrapy/Crawlee/Crawl4AI 等爬虫框架都不内置阶段耗时拆解**——这是个空白
- Rust 生态标准做法是 **tracing span**（`#[instrument]` 宏），无 subscriber 时零开销（no-op），有 subscriber 时自动记录 span duration
- wisp 已用 tracing，加 `#[instrument]` 属性即可，无需 feature gate

## 方案选择

**方案 3：纯 tracing span + 自定义 TimingLayer（Approach A）**

- 框架代码只加 `#[instrument]` 属性，函数体不变
- 无 subscriber 时 span 宏展开为 no-op，零开销
- benchmark 里用自定义 `TimingLayer` 聚合 span duration，打印百分比
- 生产用 `RUST_LOG=wisp=trace` 或接 OpenTelemetry Layer 复用同一套 span

## 设计详情

### 1. 埋点范围（6 个函数）

在 engine.rs 和 middleware/mod.rs 的 6 个关键函数加 `#[instrument]`：

```rust
// engine.rs
#[instrument(skip(ctx), fields(url = %req.url))]
pub(crate) async fn process_request(ctx: &EngineContext, req: Request) -> Option<Response>

#[instrument(skip(ctx, resp), fields(status = resp.status))]
pub(crate) async fn process_response(ctx: &EngineContext, resp: Response)

#[instrument(skip(ctx, req), fields(domain = %domain))]
async fn acquire_and_fetch(ctx: &EngineContext, req: &Request) -> (Option<Response>, Option<String>)

#[instrument(skip(ctx, req), fields(mode = ?ctx.config.fetch_mode))]
async fn fetch_dispatch(ctx: &EngineContext, req: &Request) -> (Option<Response>, Option<String>)

// middleware/mod.rs
#[instrument(skip(self, req, crawl_ctx))]
pub async fn run_request_middlewares(...)

#[instrument(skip(self, resp, crawl_ctx))]
pub async fn run_response_middlewares(...)
```

**skip 策略**：`ctx`/`req`/`resp`/`crawl_ctx` 都 skip（含大对象/引用，不需 Display），只提取关键字段到 `fields`。

**span 层级**（process_request 和 process_response 同级，由 runner 编排）：

```
process_request{url=...}
├── run_request_middlewares
└── acquire_and_fetch{domain=...}
    └── fetch_dispatch{mode=Http}

process_response{status=200}
└── run_response_middlewares
```

### 2. TimingLayer（benches/timing_layer.rs，约 60 行）

```rust
#[derive(Clone)]
pub struct TimingLayer {
    inner: Arc<Inner>,
}

struct Inner {
    create_times: Mutex<HashMap<Id, Instant>>,
    stats: Mutex<HashMap<String, (Duration, usize)>>,  // name → (total, count)
}

impl<S> Layer<S> for TimingLayer
where S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, _attrs, id: &Id, _ctx) {
        // 记 span 创建时间（不是 on_enter）
        self.inner.create_times.lock().insert(id.clone(), Instant::now());
    }

    fn on_close(&self, id: &Id, ctx) {
        // wall clock duration = now - 创建时间（含所有 await 挂起）
        if let Some(created) = self.inner.create_times.lock().remove(id) {
            let dur = created.elapsed();
            let name = ctx.span_scope(id).map(|s| s.name()).unwrap_or("");
            // 累加到 stats[name]
        }
    }
}
```

**关键设计**：用 `on_new_span`（创建时）而非 `on_enter`（进入时）记时间。async span 可能多次 enter/exit（每次 poll），但创建到关闭的 wall clock = 该阶段真实耗时（含 I/O 等待）。

**公共方法**：
- `new()` — 创建空 TimingLayer
- `reset()` — 清空 stats（每个 benchmark 级别前重置）
- `print_summary()` — 按 total duration 降序打印各阶段耗时 + 百分比 + 调用次数

### 3. benchmark 集成（global subscriber）

**关键问题**：`process_request` 通过 `tokio::spawn` 在 worker 线程执行，thread-local subscriber 抓不到。必须用 **global subscriber**。

```rust
// benches/bench.rs
use std::sync::OnceLock;
use timing_layer::TimingLayer;

static TIMING: OnceLock<TimingLayer> = OnceLock::new();

fn timing() -> &'static TimingLayer {
    TIMING.get_or_init(|| {
        let layer = TimingLayer::new();
        tracing::subscriber::set_global_default(
            tracing_subscriber::registry().with(layer.clone())
        ).ok();
        layer
    })
}

fn bench_engine_concurrent_fetch(c: &mut Criterion) {
    let timing = timing();
    for &concurrent in &[1, 4, 16] {
        timing.reset();
        // ... b.iter(|| engine.run(spider)) ...
        timing.print_summary();
    }
}
```

**OnceLock** 保证 global subscriber 只设一次，多个 benchmark 复用同一 TimingLayer。

### 输出示例

```
engine_concurrent_fetch/16 - Stage Timing:
  process_request            16.90 ms (100%)   210 calls
  acquire_and_fetch          12.30 ms (72.7%)  210 calls
  process_response            1.50 ms  (8.9%)  210 calls
  run_request_middlewares     0.80 ms  (4.7%)  210 calls
  fetch_dispatch              0.30 ms  (1.8%)  210 calls
  run_response_middlewares    0.20 ms  (1.2%)  210 calls
```

## 实现要点

1. **框架零侵入**：只加 `#[instrument]` 属性，函数体不变，不引入 feature gate
2. **零开销**：无 subscriber 时 span 宏展开为 no-op
3. **生产复用**：用户 `RUST_LOG=wisp=trace` 即可看 span 层级（或接 OpenTelemetry Layer 导出 Jaeger）
4. **benchmark 专属聚合**：TimingLayer 在 `benches/` 目录，不污染框架代码
5. **global subscriber**：解决 tokio spawn worker 线程 span 丢失问题
6. **wall clock duration**：`on_new_span` → `on_close`，含 await 挂起 = 真实阶段耗时
7. **TimingLayer Clone**：内部 Arc，可放 OnceLock + 注册到 registry

## 涉及文件

| 文件 | 改动 |
|---|---|
| `src/crawl/engine.rs` | 4 个函数加 `#[instrument]` 属性 |
| `src/crawl/middleware/mod.rs` | 2 个函数加 `#[instrument]` 属性 |
| `benches/timing_layer.rs` | 新建，TimingLayer 实现 |
| `benches/bench.rs` | engine_concurrent_fetch 集成 TimingLayer |
| `Cargo.toml` | 无改动（tracing + tracing-subscriber 已在依赖中） |

## 验证方式

1. `cargo build` — 确认 `#[instrument]` 编译通过，0 error 0 warning
2. `cargo bench --bench bench -- engine_concurrent_fetch` — 确认输出各阶段百分比
3. `cargo bench --bench bench -- parse` — 确认 parser benchmark 不受影响（无 subscriber 时零开销）
4. `cargo test --lib` — 确认 201 测试全过
5. `RUST_LOG=wisp=trace cargo run` — 确认生产环境也能看 span 输出

## 不做的事（YAGNI）

- 不加 feature gate（tracing span 无 subscriber 时已零开销）
- 不埋 `check_control_and_hook`（开销可忽略，< 1µs）
- 不埋 `spider.handle`（用户代码，不应侵入）
- 不埋 `fetch_page`（太细，fetch_dispatch 已覆盖）
- 不做 OpenTelemetry 导出（用户可自行接 Layer，框架不绑定）
