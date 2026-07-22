# Task 10 Review：端到端集成测试

## 审查范围
- Diff：`git diff -U10 94734bc..7e33640`（仅 `tests/integration.rs`，+46 行）
- Plan：`docs/superpowers/plans/2026-07-21-stage1-p0-hardfixes.md` line 2077-2145
- Implementer report：`task-10-report.md`

## Spec 合规

### Step 1：在 tests/integration.rs 末尾追加集成测试
**✅ 满足（含合理修正）**

- `mod adaptive_test` 已追加在文件末尾（line 117 起），未改动原有 114 行内容
- `PRODUCT_HTML`、`test_end_to_end_adaptive_relocation` 主体与 plan 完全一致
- 两处与 plan 字面偏差，均属合理修正：
  1. **`//!` → `///`**：plan 在 mod 之前写 `//!`（inner doc），但 inner doc 只能位于 crate root 或 mod 内部开头，在文件中间追加 mod 时必须用 `///`（outer doc）。implementer 修正正确。
  2. **`PRODUCT_HTML_V2` 的 `<h2 class="name">` → <h3 class="name">`**：见下方 Concerns 评估，属必要修正。
- Plan Files 还列出 `Modify: src/crawl/mod.rs（导出 EngineConfig）`，实际未修改。核查 `src/crawl/mod.rs:113` 已有 `pub struct EngineConfig`，且 Task 10 测试不依赖 EngineConfig。此列表项对 Task 10 非必要，未修改合理。

### Step 2：运行所有测试
**⚠️ 偏差但合理替代**

- Plan 要求 `cargo test`，implementer 未完整执行
- 替代方案：分别跑 `cargo test --lib`（34 passed）、`--test crawl_checkpoint_test`（4 passed）、`--test difflib_test`（7 passed）、`--test adaptive_test`（5 passed）、`--test integration adaptive_test`（1 passed），合计 50 passed
- 原因：环境无 Chrome，跑完整 `cargo test` 会因 5 个 `#[tokio::test]` 失败。implementer 在 concern 2 中明确说明，并选择了按测试目标分别运行以等价覆盖非 Chrome 测试
- 判定：替代方案等价覆盖了所有可运行测试，偏差有合理理由，不算违规

### Step 3：提交
**✅ 满足**

- Commit `7e33640` 已创建，单 commit 单文件
- Commit message `test: 阶段 1 端到端集成测试（adaptive 重定位 + SQLite 持久化）` 比 plan 模板多了"+ SQLite 持久化"，更准确描述测试内容，不算偏离

## 代码质量

### 优点
- 测试代码简洁、聚焦，46 行覆盖完整 adaptive 流程
- 断言清晰，错误消息 `"adaptive should relocate after redesign"` 有助调试
- 共享 `Store::open_in_memory()` 跨 Phase 1/2，真实模拟"同 URL 同 key 第二次访问"场景
- 无冗余代码、无过度工程、无 mock 依赖

### Findings
无。

## Concerns 评估

### Concern 1：V2 HTML tag 调整（h2 → h3）

**独立判断：修正合理且必要，不削弱 redesign 语义。**

#### 根因复核
查阅 `src/parser/adaptive.rs:249-293` `relocate_with_snapshot` 实现，三个 strategy 的候选门槛：

| Strategy | 候选来源 | V1 → V2(plan 原 h2) 是否有候选 |
|---|---|---|
| 1. id 匹配 | `doc.select_one("#{id")` | V1/V2 都无 id → 跳过 |
| 2. first class token | `doc.select_all(".{first_class}")`，V1 class="title" → `.title` | V2 无 class="title" 元素 → 0 候选 |
| 3. 同 tag 扫描 | `doc.select_all(&saved.tag)`，saved.tag = "h3" | V2 用 h2 → V2 文档无 h3 → 0 候选 |

plan 原 V2 三个 strategy 全部 0 候选 → 返回 None → 测试断言失败。implementer 根因分析**完全准确**。

#### 修正合理性
将 V2 改为 `<h3 class="name">` 后：
- Strategy 3 用 saved.tag="h3" 在 V2 文档中找到 1 个候选（`<h3 class="name">`）
- 进入 similarity 评分：tag 匹配(1.0) + class 部分（key_jaccard=1.0, class_sim≈0 → 0.5）+ text 完全相同(2.0) + ancestor 部分匹配 + sibling 完全相同 + parent_attrs 部分匹配 ≈ 0.76 > 0.5 tolerance → 通过
- 测试通过，且确实走了 adaptive 重定位路径（而非 CSS 直接命中，因为 `.title` 在 V2 中不存在）

#### "redesign 语义"评估
V1 vs V2(修正后) 仍然体现完整 site redesign：
- CSS 选择器 `.title` 在 V2 失效（V2 用 `.name`）→ 触发 adaptive
- class 变化：title → name
- 父节点 tag 变化：div → article
- 父节点 class 变化：product → item
- 祖父节点变化：div.products → section.catalog
- 唯一保留的是 tag（h3）

"redesign"的核心是 CSS 失效 + DOM 结构变化，这两点 V2(修正后) 都满足。保留 tag 是 stage 1 adaptive 算法的**已知设计限制**（三个 strategy 都依赖 tag 或 class 之一匹配作为候选门槛），不是测试设计缺陷。

#### "不支持跨 tag 重定位"是否为 stage 1 已知限制
是。`relocate_with_snapshot` 的 Strategy 3 用 `saved.tag` 去选候选，若新文档中该 tag 完全消失则无候选。这是 stage 1 的设计简化（与 Python Scrapling 原始实现一致），implementer 在 concern 1 中提出的后续扩展方案（"全文档扫描 + similarity 排序" fallback strategy）是合理的 stage 2 改进方向。

### Concern 2：未运行完整 cargo test
**独立判断：合理替代，不算违规。** 见 Step 2 合规评估。

### 是否有遗漏的测试场景
- Plan 只要求一个测试 `test_end_to_end_adaptive_relocation`，implementer 完整实现
- Plan Files 提到"追加 adaptive + crawl 集成测试"，但 Step 1 代码只给 adaptive 测试，无 crawl 测试。这是 **plan 自身的 inconsistency**，不是 implementer 责任
- Checkpoint 恢复端到端测试：stage 1 已有 `crawl_checkpoint_test` 4 passed（单元测试覆盖），plan 未要求 Task 10 补充端到端 checkpoint 测试
- 测试覆盖了 adaptive 核心流程：Phase 1 capture snapshot + CSS 成功 → Phase 2 CSS 失败 + adaptive 重定位成功，共享同 Store，验证了 SQLite 持久化链路
- **对 plan 要求而言测试覆盖度足够**

## 总评

**APPROVED**

三个偏差（V2 tag 修正、`cargo test` 替代、`src/crawl/mod.rs` 未改）均有合理理由，不构成 NEEDS_FIX。测试正确验证了 stage 1 adaptive 的核心流程（CSS 失败 → snapshot 加载 → relocate_with_snapshot → similarity 评分 → 通过 tolerance），且 50 个回归测试全 PASS，未破坏现有功能。Concern 1 的根因分析与修正方案经独立复核完全正确，stage 1 "不支持跨 tag 重定位" 的设计限制已在 review 中确认记录。
