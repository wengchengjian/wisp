# 性能基准标准

## 1. 目标

用同一套可复现的本地小说站流程量化 wisp 的抓取性能，覆盖：

- 端到端吞吐（pages/s、items/s、ms/page）
- 引擎阶段耗时占比（request/response 中间件、handle、parse、fetch）
- 配置对照（Auto / Http / 最小传输链 / 缓存回放）
- 持续运行时的内存与 CPU 表现

本地服务器排除真实站点网络波动，结果在同一台机器上可对比。

## 2. 标准负载

- 首页：10 本书链接
- 详情页：每本书 30 个章节链接
- 章节页：8KB 中文正文
- 请求数：311（1 首页 + 10 详情 + 300 章节）
- 并发：4
- 延迟：0
- 缓存：默认关闭
- 模式：默认 Auto（DynamicUpgrade 关闭）
- 传输中间件：UA 轮换 + 固定 headers + CookieChallenge

## 3. 指标

| 指标 | 说明 |
| --- | --- |
| 单轮耗时 | criterion 测得的每轮墙钟时间 |
| pages/s | 311 页 / 单轮秒数 |
| items/s | 300 item / 单轮秒数 |
| ms/page | 单轮毫秒 / 311 |
| 阶段占比 | `WISP_TIMING=1` 下各 tracing span 聚合占比 |
| 缓存命中 | 开启缓存后第二次起 pages_crawled=0、cache_hits>0 |
| 峰值内存 | 持续运行 profiler 时的 WorkingSet/PrivateBytes 峰值 |

## 4. 当前基线（2026-08-01，master 合并 crate 拆分 + arch-refactor 后，Windows + profiling/release）

### novel_flow

```text
novel_flow/multi_spider_10books
    time: [23.259 ms 25.932 ms 28.433 ms]
```

折算：

| 指标 | 值 |
| --- | --- |
| pages/s | 约 12,000 |
| items/s | 约 11,600 |
| ms/page | 约 0.083 |

### 阶段占比（WISP_TIMING=1）

> 阶段占比为 2026-08-01 短采样（`--sample-size 10 --measurement-time 1 --warm-up-time 1`）结果。

```text
process_request       100.0%
  fetch_dispatch      96.7%
  fetch               95.4%
  run_request_middlewares   0.6%
process_response      约 60.9%（与 process_request 嵌套，非可加占比）
  run_response_middlewares  11.6%
  spider.handle       22.8%
```

### 非网络微基准（2026-08-01，criterion 短采样）

| 路径 | 耗时 | 说明 |
| --- | --- | --- |
| parse/10KB | 约 99 µs | HTML 解析 |
| parse/100KB | 约 1.35 ms | HTML 解析 |
| parse/1MB | 约 14.2 ms | HTML 解析 |
| text_extraction（100KB 页面） | 约 156 µs | `.item` 文本提取 |
| scheduler_push_1000 | 约 258 µs | 单线程 push |
| scheduler_concurrent_push_4x250 | 约 955 µs | 4 任务并发 push |

结论：fetch 仍是绝对主导（约 95%）。非网络热点按占比为
`spider.handle`（约 22.8%，主要为 HTML parse/text）> `run_response_middlewares`
（约 11.6%）> request 中间件（0.6%）；scheduler push 不是瓶颈。

### 中间件占比（WISP_TIMING=1，Auto 模式 novel_flow 短采样）

| 中间件 | 平均耗时 | 占 process_request |
| --- | --- | --- |
| response:StealthUpgradeMiddleware | 2.00 ms | 4.1% |
| request:HeadersMiddleware | 0.18 ms | 0.4% |
| request:UaRotationMiddleware | 0.13 ms | 0.3% |
| response:HeadersMiddleware | 0.10 ms | 0.2% |
| response:BlockedRetryMiddleware | 0.10 ms | 0.2% |
| 其余（CookieChallenge/Retry/Headers 等） | 各 <0.1 ms | 各 <0.2% |

结论：快速短路后 `StealthUpgradeMiddleware` 从 5.83 ms（11.8%）降到 2.00 ms（4.1%），
占 `run_response_middlewares`（3.64 ms）约 55%。收益来自：header 检查提前、单次遍历
匹配全部 body 特征、空 body 短路，以及非 HTML Content-Type 直接跳过。

### 配置对照（novel_flow_variants）

运行 `cargo bench --bench bench -- novel_flow_variants` 获取：

- `auto_default`：Auto + 传输中间件
- `http_with_transport`：Http + 传输中间件
- `http_minimal`：Http，无 UA/headers/CookieChallenge
- `http_cached_replay`：Http + MemoryStore 缓存稳定态回放

当前基线：

| 变体 | 时间 | 说明 |
| --- | --- | --- |
| auto_default | 约 28.1 ms | Auto 模式仍保留每请求 CF/规则快速检查 |
| http_with_transport | 约 22.8 ms | 明确 Http 后省掉 Auto 嗅探 |
| http_minimal | 约 23.7 ms | 传输中间件本身开销约 2-3% |
| http_cached_replay | 约 9.2 ms | 缓存回放约是完整抓取吞吐的 2.8 倍 |

> 2026-08-01 完整基准：默认 criterion 配置（warm-up 3s、measurement 5s、sample 10）。

## 4.5 当前基线（2026-08-04，master 合并 StealthStrategy 迁移 + MCP 薄壳化 + multi-spider 后，Windows + release profile）

运行方式：`cargo bench --bench bench --profile release -- --save-baseline 2026-08-04`。

> 与 08-01 基线对比：**scheduler / parser 微基准正常**（scheduler_push 几乎一致、text_extraction 更快），
> 但 **novel_flow 系列整体变慢 30%~2.7x**。该差异为 08-01 之后多次重构（crate split、
> multi-spider refactor、typed item delivery、引擎 item 单 seam 化）的累积，非本次 stealth 迁移
> 单独引入（本次改动不触及 novel_flow 的 HTTP 路径）。**待确认是否真实回归**，见 §7 已知优化边界。

### novel_flow（release）

```text
novel_flow/multi_spider_10books
    time: [32.475 ms 34.184 ms 37.416 ms]
```

折算：

| 指标 | 值 |
| --- | --- |
| pages/s | 约 9,000 |
| items/s | 约 8,800 |
| ms/page | 约 0.110 |

### 非网络微基准（release）

| 路径 | 08-01 | 08-04 | 变化 |
| --- | --- | --- | --- |
| text_extraction（100KB 页面） | 约 156 µs | 148.95 µs | ✅ 更快 |
| scheduler_push_1000 | 约 258 µs | 272.99 µs | ✅ 几乎一致 (+6%) |
| scheduler_concurrent_push_4x250 | 约 955 µs | 964.90 µs | ✅ 几乎一致 (+1%) |

### 配置对照（release，novel_flow_variants）

| 变体 | 08-01 | 08-04 | 变化 |
| --- | --- | --- | --- |
| auto_default | 约 28.1 ms | 38.699 ms | ⚠️ 慢 ~38% |
| http_with_transport | 约 22.8 ms | 37.684 ms | ⚠️ 慢 ~65% |
| http_minimal | 约 23.7 ms | 64.796 ms | ⚠️ 慢 ~2.7x |
| http_cached_replay | 约 9.2 ms | 11.728 ms | ⚠️ 慢 ~27% |

### 4.6 热点路径 Profiling（2026-08-04，samply + TimingLayer，本地 HTTP/1.1 小说站）

**结论：框架逻辑无回归，热点仍是 fetch（网络路径）。** 08-04 记录的
novel_flow 30%~2.7x 变慢处于测量噪声带内（CPU 频率缩放），非本次重构引入的框架逻辑回归。

证据：

1. **阶段耗时占比与 08-01 基线一致**（`WISP_TIMING=1`，novel_flow/multi_spider）：

   | 阶段 | 占比 | 08-01 基线 |
   | --- | --- | --- |
   | fetch_with_retry | 约 94% | 约 95% |
   | fetch | 约 92% | 约 95% |
   | spider.handle | 约 17-19% | 约 23% |
   | run_response_middlewares | 约 6.1-6.2% | 约 6.5% |
   | StealthUpgradeMiddleware | 约 3.3-3.4% | 约 4.1% |

   handle/parse/middleware 等框架逻辑占比基本未变，不是回归来源。

2. **确定性微基准稳定**：scheduler_push（+10% vs 08-04）、scheduler_concurrent（-18.5%）、
   text_extraction 均落在噪声带内。scheduler 不是瓶颈。

3. **samply 热点符号为符号解析伪影**：本地服务器为纯 HTTP/1.1 TCP（无 TLS、无 ALPN），
   wreq 默认 `HttpVersionPref::All`，对 `http://` 明文连接只能走 HTTP/1.1（h2c prior-knowledge
   仅在 `http2_only` 时启用）。因此 profile 中解析出的 `recv_data → schedule_implicit_reset →
   reclaim_reserved_capacity`（h2 栈）与 `ReverseInner::try_search_full`（regex）均非真实热点，
   是 Windows PDB 符号解析失败（`fun_13a690` 占 85% 叶帧）导致的误归因。

4. **criterion 对比噪声巨大**：同一环境 novel_flow 方差达 ±30%（p=0.49），
   `http_minimal` 跨 session 波动 39~64ms。绝对墙钟不稳定，不能据此判定回归。

**优化边界**：真实热点是 fetch（网络），其成本由 wreq 的 tower 层
（ConfigServiceLayer/TimeoutLayer/FollowRedirectLayer，`ClientBuilder::build` 无条件叠加，无公开开关）
与连接复用决定。要进一步压低 fetch 开销需绕开 wreq 高层构造（直接使用 connector/service 或 hyper），
属于后续大改，见 §7。

## 5. 中间件清单

默认链（Auto、缓存关闭、DynamicUpgrade 关闭、启用 UA/headers/cookie）：

| 阶段 | 中间件 | 优先级 |
| --- | --- | --- |
| Request | DepthLimit | 5 |
| Request | Headers | 10 |
| Request | UaRotation | 20 |
| Response | StealthUpgrade（仅 Auto） | 45 |
| Response | CookieChallenge（启用时） | 50 |
| Response | BlockedRetry | 80 |
| Error | Retry | 90 |

开启 `cache_store` 时增加 Cache（3，请求/响应双向）；开启
`dynamic_upgrade(true)` 时增加 DynamicUpgrade（40，Response）。
Http/Dynamic/Stealth 模式不注入 StealthUpgrade/DynamicUpgrade。

运行时可用 `RUST_LOG=wisp::crawl::middleware=trace` 查看实际注入链：
`MiddlewareChain::run_init` 会输出 `middleware chain: A -> B -> C`。

## 6. 运行方式

```bash
# 单轮基准
cargo bench --bench bench -- novel_flow

# 配置对照
cargo bench --bench bench -- novel_flow_variants

# 阶段耗时占比
WISP_TIMING=1 cargo bench --bench bench -- novel_flow

# 持续负载（可调 NOVEL_* 环境变量）
cargo run --profile profiling --example novel_profiler

# 火焰图
cargo flamegraph --profile profiling --example novel_profiler
```

## 7. 已知优化边界

- `blocked_reason` 已改为 64KB 头部窗口 + ASCII case-insensitive 字节匹配，
  不再对完整 body 做 UTF-8 解码和小写分配。
- `build_fetch_response` 的 header 转换已预分配 HashMap 容量；
  `Page::content_text` 改为先拼接再收尾，减少逐行分支。
- wreq 6.0.0-rc.29 的 `ClientBuilder::build` 无条件叠加 RetryLayer/FollowRedirectLayer/
  ConfigServiceLayer/TimeoutLayer，没有公开开关跳过；绕开需要换更底层构造（直接使用
  wreq connector/service 或 hyper），属于后续大改，本次仅完成调研并记录。
- woven-html 的 `parse` 需要持有 `String`，HTML 解析本身无法完全零拷贝；当前已做到
  `decode_borrowed` 借用 + `from_html_owned` 移交，避免一次整页复制。
- wreq 默认自带每请求 2 次协议重试层，与 Engine 重试重复；已通过
  `retry::Policy::never()` 关闭，由 wisp 统一管理重试。
- mio/winsock 是 tokio/OS 网络栈，wisp 能控制的是连接复用、header 构造、缓存和中间件数量。
- 响应缓存默认关闭；需要重复抓取/回放时用 `.cache_store(MemoryStore/FileStore)` 显式开启。
