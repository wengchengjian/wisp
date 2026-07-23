### Task 6: 最终回归验证与清理

**Files:**
- 全量测试
- `docs/superpowers/plans/2026-07-23-p1-optimization.md`（本文件，标记完成）

**Interfaces:**
- 无新增接口

- [ ] **Step 1: 全量 lib + 集成测试**

Run: `cargo test --lib`
Expected: 206+ 测试全绿（可能因新增单元测试略增）。

Run: `cargo test --test p1_status_codes_test --test p1_proxy_clients_test --test p1_scheduler_test --test p1_meta_persistence_test --test p0_autoscale_test --test p0_dashmap_test --test engine_infra_test --test crawl_concurrency_test --test multi_spider_test --test builder_api_test --test cr_fix_engine_test`
Expected: 全部 PASS。

- [ ] **Step 2: 验证 clippy 无新警告**

Run: `cargo clippy --lib 2>&1 | grep "generated.*warnings"`
Expected: `28 warnings`（与 P0 完成后基线一致，不新增）。

- [ ] **Step 3: 标记 plan 完成**

在 plan 文件中将所有 `- [ ]` 改为 `- [x]`：

Run: `sed -i 's/^- \[ \]/- [x]/g' docs/superpowers/plans/2026-07-23-p1-optimization.md`

- [ ] **Step 4: 提交 plan 完成标记**

`docs/` 已被本地 `.gitignore`（工作区修改），plan 文件为本地工作文件，无法 commit。此 Step 为 no-op，跳过提交，仅本地标记完成。

---

## Self-Review

### 1. Spec coverage

| Spec 项 | Plan Task |
|---|---|
| P1-1 status_codes/proxy_clients 每请求锁 | Task 2 (status_codes DashMap) + Task 3 (proxy_clients DashMap) |
| P1-2 Scheduler 单 Mutex<BinaryHeap> | Task 4 (seen DashSet + heap 独立 Mutex) |
| P1-5 Method 枚举与转换重复 | Task 1 (Method::as_str + 替换 3 处) |
| P1-7 SpiderRequest.meta 不持久化 | Task 5 (meta_serde 自定义 serde) |
| P1-3 反检测能力薄弱 | 不在本计划（大特性，单独 spec） |
| P1-4 Storage 后端单一 | 不在本计划（大特性，单独 spec） |
| P1-6 双轨重试逻辑 | 不在本计划（核心抓取改动大、回归风险高） |

覆盖 4/7 P1 项，其余 3 项按风险/规模显式排除，已在 plan 头部说明。

### 2. Placeholder scan

无 TBD/TODO/"实现细节后补"。每步含完整代码或精确命令。

### 3. Type consistency

- `record_status`: Task 2 定义为 `pub fn record_status(stats: &Arc<SpiderStats>, status: u16)`（同步），Step 5 移除调用点 `.await`。Task 2 Step 8 re-export `pub use engine::record_status;`。一致。
- `fetch_page_inner`: Task 3 定义为 `pub async fn fetch_page_inner(..., proxy_clients: &dashmap::DashMap<String, Arc<Client>>)`, Step 8 re-export。一致。
- `status_codes_snapshot`: Task 2 Step 3 定义为 `pub fn status_codes_snapshot(&self) -> HashMap<u16, usize>`，Task 2 Step 6/7 调用。一致。
- `Scheduler`: Task 4 拆分后字段 `heap`/`seen_exact`/`seen_fp`/`strategy`，公开方法签名不变。一致。
- `meta_serde`: Task 5 Step 3 定义 `serialize`/`deserialize`，Step 4 `#[serde(with = "meta_serde")]` 引用。一致。

### 执行顺序

Task 1（最小、无依赖）→ Task 2（status_codes）→ Task 3（proxy_clients，依赖 Task 2 的 re-export 位置合并）→ Task 4（Scheduler，独立）→ Task 5（meta，独立）→ Task 6（回归）。

Task 3 Step 8 与 Task 2 Step 8 都修改 `src/crawl/mod.rs` 的 re-export 区，按顺序执行时 Task 3 合并为 `pub use engine::{record_status, fetch_page, fetch_page_inner};`（覆盖 Task 2 的单行）。
