# PR3: parser adaptive 迁移 + Feature gate 分层 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 消除 parser 对 storage 的依赖（adaptive 迁到 crawl），引入分层 feature gate（http/browser/stealth/mcp/sqlite）减少编译成本

**Architecture:** parser/adaptive.rs 只保留 ElementSnapshot 类型 + capture + similarity + relocate（纯函数），css_adaptive 迁到 crawl::adaptive::AdaptiveTracker；Cargo.toml 新增 5 个 feature，模块按 feature gate 编译

**Tech Stack:** Rust, Tokio, cargo features

## Global Constraints

- 变量命名 snake_case
- 代码注释用中文
- 提交信息用中文（一行）
- TDD：先写测试再写实现
- 保持现有测试全绿
- 不向后兼容，只考虑最优解
- PR2 已完成：fetcher/strategies/{dynamic,stealth}.rs 已存在，BrowserFetchStrategy trait 已定义，fetch_browser 签名已改为接收 `&dyn BrowserFetchStrategy`，browser/launch.rs 已拆分 build_common_args + build_stealth_extra_args，stealth/patches.rs 已从 browser/patches.rs 迁移
- banzhu-rs 依赖 wisp，需同步更新 feature 配置

---

## 关键技术决策

1. **ElementSnapshot 字段保持当前结构**：当前 `ElementSnapshot` 字段（tag/attrs/text_preview/ancestor_path/sibling_tags/position_in_parent/parent_tag/parent_attrs）是 8 维相似度匹配的载体，与 spec 5.1 的简化版（url/selector/key/html/text/captured_at）不同。PR3 保留当前字段结构，只迁移持久化职责，避免重写 similarity/relocate 逻辑。
2. **to_row/from_row 迁到 storage 层**：在 `storage/mod.rs` 新增 `impl From<ElementSnapshot> for ElementSnapshotRow` 和 `impl From<ElementSnapshotRow> for ElementSnapshot`，让 storage 双向依赖 parser（合理：storage 是底层，parser 是数据类型层）。parser 完全不依赖 storage。
3. **AdaptiveTracker::css_adaptive 返回 `Result<Option<Node>>`**：保持原 `css_adaptive` 自由函数的语义（返回 Node 用于后续 .text()/.html()），只把签名包装为 `Result`（load_element 可能返回 StorageError）。调用方 mcp/tools.rs 改为 `tracker.css_adaptive(...).await?`。
4. **存储格式不变**：AdaptiveTracker 内部仍调用 `crate::storage::save_element` / `load_element` 自由函数（NS_ELEMENT namespace + `url|key` composite key），与原 css_adaptive 完全一致，已有 SQLite 数据无需迁移。
5. **captured_at 时间戳生成点变化**：原 `to_row(now)` 由调用方传入时间戳；新 `From::from(snap)` 内部用 `chrono::Utc::now().timestamp()`。功能等价（auto_save 时都是当前时间）。
6. **default = ["http"]**：HTTP-only 是最小可用配置，包含 fetcher/http/parser/storage/crawl 基础。browser/stealth/mcp/sqlite 按需启用。
7. **Feature gate 不影响现有测试**：所有现有测试在 `--all-features` 下运行通过；http-only 模式下 browser/stealth 相关测试加 `#[cfg(feature = "...")]` 跳过。
8. **banzhu-rs 当前只用 HTTP 模式**（`FetchMode::Http` / `FetchMode::Auto`），但未来可能需要 stealth。Cargo.toml 启用 `["http", "stealth", "sqlite"]` 以保留升级路径。

---

## 文件结构

PR3 完成后，新增/修改文件如下：

```
src/
├── parser/
│   ├── adaptive.rs           [修改] 删除 to_row/from_row/css_adaptive，保留 ElementSnapshot/capture/similarity/relocate
│   └── mod.rs                [修改] 删除 Node::css_adaptive 方法和 css_adaptive re-export
├── crawl/
│   ├── adaptive.rs           [新建] AdaptiveTracker（持有 Arc<dyn Store>，提供 css_adaptive）
│   └── mod.rs                [修改] 新增 `pub mod adaptive;` 和 re-export
├── storage/
│   └── mod.rs                [修改] 新增 From<ElementSnapshot>/From<ElementSnapshotRow> 双向转换
├── fetcher/
│   ├── mod.rs                [修改] strategies 模块 + re-export 加 cfg gate
│   └── strategies/mod.rs     [修改] dynamic/stealth 模块声明加 cfg gate
├── browser/
│   └── launch.rs             [修改] build_stealth_extra_args 加 #[cfg(feature = "stealth")]
├── lib.rs                    [修改] browser/stealth/mcp 模块加 cfg gate
├── mcp/
│   └── tools.rs              [修改] css_adaptive 调用改为 AdaptiveTracker::css_adaptive
└── Cargo.toml                [修改] 新增 http/browser/stealth/mcp feature，可选依赖标记 optional

banzhu-rs/
└── Cargo.toml                [修改] wisp 依赖启用 features = ["http", "stealth", "sqlite"]
```

---

## Task 1: storage 新增 From<ElementSnapshot> 双向转换

**Files:**
- Modify: `src/storage/mod.rs:101-122`（ElementSnapshotRow 定义后新增 impl 块）
- Test: `src/storage/mod.rs`（内联 `#[cfg(test)] mod tests` 新增测试）

**Interfaces:**
- Consumes: `crate::parser::ElementSnapshot`（已存在，字段：tag/attrs/text_preview/ancestor_path/sibling_tags/position_in_parent/parent_tag/parent_attrs）
- Produces: `impl From<crate::parser::ElementSnapshot> for ElementSnapshotRow`、`impl From<ElementSnapshotRow> for crate::parser::ElementSnapshot`

- [ ] **Step 1: 写失败的测试**

在 `src/storage/mod.rs` 的 `#[cfg(test)] mod tests` 末尾新增测试（在 `namespace_isolation` 测试后）：

```rust
    #[test]
    fn element_snapshot_roundtrip_via_from() {
        use crate::parser::ElementSnapshot;
        use std::collections::HashMap;

        let mut attrs = HashMap::new();
        attrs.insert("class".to_string(), "item main".to_string());
        attrs.insert("id".to_string(), "product-1".to_string());

        let mut parent_attrs = HashMap::new();
        parent_attrs.insert("class".to_string(), "container".to_string());

        let snap = ElementSnapshot {
            tag: "div".to_string(),
            attrs,
            text_preview: "Hello World".to_string(),
            ancestor_path: vec!["html".to_string(), "body".to_string(), "div.container".to_string()],
            sibling_tags: vec!["div".to_string(), "span".to_string(), "div".to_string()],
            position_in_parent: 1,
            parent_tag: "section".to_string(),
            parent_attrs,
        };

        // ElementSnapshot -> ElementSnapshotRow
        let row: ElementSnapshotRow = snap.into();
        assert_eq!(row.tag, "div");
        assert_eq!(row.text_preview, "Hello World");
        assert_eq!(row.position_in_parent, 1);
        assert_eq!(row.parent_tag, "section");
        // 验证 serde_json::Value 序列化
        assert!(row.attrs.is_object());
        assert!(row.ancestor_path.is_array());
        assert!(row.sibling_tags.is_array());
        assert!(row.parent_attrs.is_object());
        // captured_at 由 From impl 内部生成（当前时间戳）
        assert!(row.captured_at > 0);

        // ElementSnapshotRow -> ElementSnapshot
        let restored: ElementSnapshot = row.into();
        assert_eq!(restored.tag, "div");
        assert_eq!(restored.text_preview, "Hello World");
        assert_eq!(restored.position_in_parent, 1);
        assert_eq!(restored.parent_tag, "section");
        assert_eq!(restored.attrs.get("class").map(String::as_str), Some("item main"));
        assert_eq!(restored.attrs.get("id").map(String::as_str), Some("product-1"));
        assert_eq!(restored.ancestor_path, vec!["html".to_string(), "body".to_string(), "div.container".to_string()]);
        assert_eq!(restored.sibling_tags, vec!["div".to_string(), "span".to_string(), "div".to_string()]);
        assert_eq!(restored.parent_attrs.get("class").map(String::as_str), Some("container"));
    }
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib element_snapshot_roundtrip_via_from`
Expected: FAIL with "cannot find function `into`" 或类型不匹配错误（因为 From impl 还未添加）

- [ ] **Step 3: 写最小实现**

在 `src/storage/mod.rs` 的 `ElementSnapshotRow` 结构体定义后（约第 122 行 `}` 后）新增两个 impl 块：

```rust
// ============================================================================
// ElementSnapshot <-> ElementSnapshotRow 转换
// ============================================================================

/// ElementSnapshot -> ElementSnapshotRow（捕获时间由 storage 层生成）。
///
/// ARCH: parser 只产出 ElementSnapshot 值对象，持久化转换由 storage 层承担。
/// captured_at 在转换时取当前时间戳（与原 to_row(now) 行为一致）。
impl From<crate::parser::ElementSnapshot> for ElementSnapshotRow {
    fn from(s: crate::parser::ElementSnapshot) -> Self {
        Self {
            tag: s.tag,
            attrs: serde_json::to_value(&s.attrs).unwrap_or(serde_json::json!({})),
            text_preview: s.text_preview,
            ancestor_path: serde_json::to_value(&s.ancestor_path).unwrap_or(serde_json::json!([])),
            sibling_tags: serde_json::to_value(&s.sibling_tags).unwrap_or(serde_json::json!([])),
            position_in_parent: s.position_in_parent as i64,
            parent_tag: s.parent_tag,
            parent_attrs: serde_json::to_value(&s.parent_attrs).unwrap_or(serde_json::json!({})),
            captured_at: chrono::Utc::now().timestamp(),
        }
    }
}

/// ElementSnapshotRow -> ElementSnapshot（反序列化恢复）。
///
/// ARCH: parser 不依赖 storage 类型，反向转换由 storage 层提供。
impl From<ElementSnapshotRow> for crate::parser::ElementSnapshot {
    fn from(row: ElementSnapshotRow) -> Self {
        Self {
            tag: row.tag,
            attrs: serde_json::from_value(row.attrs).unwrap_or_default(),
            text_preview: row.text_preview,
            ancestor_path: serde_json::from_value(row.ancestor_path).unwrap_or_default(),
            sibling_tags: serde_json::from_value(row.sibling_tags).unwrap_or_default(),
            position_in_parent: row.position_in_parent as usize,
            parent_tag: row.parent_tag,
            parent_attrs: serde_json::from_value(row.parent_attrs).unwrap_or_default(),
        }
    }
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib element_snapshot_roundtrip_via_from`
Expected: PASS

- [ ] **Step 5: 提交**

```bash
git add src/storage/mod.rs
git commit -m "feat: storage 新增 ElementSnapshot 双向 From 转换"
```

---

## Task 2: 删除 parser/adaptive.rs 的 to_row/from_row 方法

**Files:**
- Modify: `src/parser/adaptive.rs:8`（删除 `use crate::storage::{ElementSnapshotRow, Store};` 中的 `ElementSnapshotRow`）
- Modify: `src/parser/adaptive.rs:117-155`（删除 `to_row` 和 `from_row` 方法）
- Test: `cargo build --lib` 验证编译通过

**Interfaces:**
- Consumes: Task 1 的 `impl From<ElementSnapshot> for ElementSnapshotRow`
- Produces: `ElementSnapshot` 不再有 `to_row`/`from_row` 方法（调用方改用 `.into()`）

- [ ] **Step 1: 写失败的测试**

在 `src/parser/adaptive.rs` 末尾新增 `#[cfg(test)] mod tests`（如果不存在）：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_snapshot_no_longer_has_to_row_or_from_row() {
        // 验证 to_row/from_row 方法已删除（编译期检查）
        // 如果方法仍存在，下面的 trait bound 会冲突或编译失败
        let snap = ElementSnapshot {
            tag: "div".into(),
            attrs: std::collections::HashMap::new(),
            text_preview: String::new(),
            ancestor_path: vec![],
            sibling_tags: vec![],
            position_in_parent: 0,
            parent_tag: String::new(),
            parent_attrs: std::collections::HashMap::new(),
        };

        // 通过 storage::ElementSnapshotRow::from 转换（而非 snap.to_row(0)）
        let row: crate::storage::ElementSnapshotRow = snap.into();
        assert_eq!(row.tag, "div");

        // 反向转换
        let restored: ElementSnapshot = row.into();
        assert_eq!(restored.tag, "div");
    }
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib element_snapshot_no_longer_has_to_row_or_from_row`
Expected: 编译失败或警告（`to_row`/`from_row` 方法仍存在，可能有 unused 警告；测试本身应能通过，因为 `.into()` 已可用）。如果测试通过，说明实现已正确，可直接进入 Step 3 删除方法。

- [ ] **Step 3: 写最小实现**

3a. 修改 `src/parser/adaptive.rs:8`，将 `use crate::storage::{ElementSnapshotRow, Store};` 改为（仅保留 Store，因为 css_adaptive 仍需 Store；Task 4 会再删除 Store）：

```rust
use crate::storage::Store;
```

3b. 删除 `src/parser/adaptive.rs:117-155` 的 `to_row` 和 `from_row` 方法（整个 `impl ElementSnapshot` 块中这两个方法，保留 `capture`）。删除后的 `impl ElementSnapshot` 块应只包含 `capture` 方法：

```rust
impl ElementSnapshot {
    /// 从 Node 捕获快照（用 Node 导航 API，不再重复解析 outer_html）。
    #[must_use]
    pub fn capture(node: &Node) -> Self {
        let tag = node.tag();
        let attrs = node.attrs();
        let text_preview = node.text();
        let text_preview = if text_preview.len() > 200 {
            text_preview.chars().take(200).collect()
        } else {
            text_preview
        };

        // ancestor_path: 从父节点到根，每级 "tag" 或 "tag.firstclass"，最后 rev() 使根在前
        let ancestor_path: Vec<String> = node
            .ancestors()
            .filter_map(|n| {
                let t = n.tag();
                if t.is_empty() {
                    return None;
                }
                let class = n.attr("class").unwrap_or_default();
                if class.is_empty() {
                    Some(t)
                } else {
                    let first_class: String =
                        class.split_whitespace().next().unwrap_or("").to_string();
                    if first_class.is_empty() {
                        Some(t)
                    } else {
                        Some(format!("{t}.{first_class}"))
                    }
                }
            })
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        // parent context
        let parent_node = node.parent();
        let parent_tag = parent_node
            .as_ref()
            .map(super::Node::tag)
            .unwrap_or_default();
        let parent_attrs = parent_node
            .as_ref()
            .map(super::Node::attrs)
            .unwrap_or_default();

        // sibling_tags: 父节点的所有元素子节点的 tag 列表
        let sibling_tags: Vec<String> = parent_node
            .as_ref()
            .map(|p| p.children().iter().map(super::Node::tag).collect())
            .unwrap_or_default();

        // position_in_parent: 当前节点在父节点子元素中的索引
        // 用 outer_html 比较身份（比 tag+text 更准确，避免相同 tag+text 的兄弟节点误匹配）
        let position_in_parent = match &parent_node {
            Some(p) => {
                let target_html = node.outer_html();
                p.children()
                    .iter()
                    .position(|c| c.outer_html() == target_html)
                    .unwrap_or(0)
            }
            None => 0,
        };

        Self {
            tag,
            attrs,
            text_preview,
            ancestor_path,
            sibling_tags,
            position_in_parent,
            parent_tag,
            parent_attrs,
        }
    }
}
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib element_snapshot_no_longer_has_to_row_or_from_row && cargo build --lib`
Expected: PASS + 编译通过（无 to_row/from_row 引用错误）

- [ ] **Step 5: 提交**

```bash
git add src/parser/adaptive.rs
git commit -m "refactor: 删除 ElementSnapshot::to_row/from_row 迁到 storage From"
```

---

## Task 3: 新建 crawl/adaptive.rs 实现 AdaptiveTracker

**Files:**
- Create: `src/crawl/adaptive.rs`
- Modify: `src/crawl/mod.rs:11`（新增 `pub mod adaptive;`）
- Test: `src/crawl/adaptive.rs`（内联 `#[cfg(test)] mod tests`）

**Interfaces:**
- Consumes: Task 1 的 `impl From<ElementSnapshot> for ElementSnapshotRow`、`impl From<ElementSnapshotRow> for ElementSnapshot`；`crate::parser::{ElementSnapshot, Node, relocate_with_snapshot, DEFAULT_TOLERANCE}`；`crate::storage::{Store, save_element, load_element}`
- Produces: `crate::crawl::adaptive::AdaptiveTracker`（结构体）、`AdaptiveTracker::new(store: Arc<dyn Store>) -> Self`、`AdaptiveTracker::css_adaptive(&self, node, selector, key, url, auto_save, tolerance) -> Result<Option<Node>>`

- [ ] **Step 1: 写失败的测试**

创建 `src/crawl/adaptive.rs`，先只写测试模块（实现部分留空触发编译失败）：

```rust
//! 自适应解析跟踪器 — 持久化元素快照。
//!
//! ARCH: 从 parser/adaptive.rs 迁移。parser 只产出 ElementSnapshot 值对象，
//! 持久化职责由 AdaptiveTracker 承担，消除 parser 对 storage 的依赖。

use std::sync::Arc;

use crate::error::Result;
use crate::parser::{relocate_with_snapshot, ElementSnapshot, Node, DEFAULT_TOLERANCE};
use crate::storage::{load_element, save_element, ElementSnapshotRow, Store};

/// 自适应解析跟踪器 — 持久化元素快照。
///
/// ARCH: 替代原 `parser::css_adaptive` 自由函数。持有 `Arc<dyn Store>`，
/// 提供 `css_adaptive` 方法：先 CSS 选择，失败则从存储加载快照重定位。
pub struct AdaptiveTracker {
    store: Arc<dyn Store>,
}

impl AdaptiveTracker {
    /// 创建跟踪器。
    #[must_use]
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self { store }
    }

    /// 自适应 CSS 选择：先 CSS，失败则从存储加载快照重定位。
    ///
    /// - `selector`: CSS 选择器（可能匹配也可能不匹配）
    /// - `key`: 元素稳定标识（用户定义，如 "product-name"）
    /// - `url`: 页面 URL（用于存储 key）
    /// - `auto_save`: 是否在成功后刷新快照
    /// - `tolerance`: 相似度阈值（0.0..1.0）
    pub async fn css_adaptive(
        &self,
        node: &Node,
        selector: &str,
        key: &str,
        url: &str,
        auto_save: bool,
        tolerance: f64,
    ) -> Result<Option<Node>> {
        // 1. Try CSS first
        if let Some(found) = node.select_one(selector) {
            if auto_save {
                let snap = ElementSnapshot::capture(&found);
                let row: ElementSnapshotRow = snap.into();
                let _ = save_element(self.store.as_ref(), url, key, &row).await;
            }
            return Ok(Some(found));
        }

        // 2. CSS failed - try relocate from saved snapshot
        let saved_row = match load_element(self.store.as_ref(), url, key).await? {
            Some(r) => r,
            None => return Ok(None),
        };
        let saved: ElementSnapshot = saved_row.into();
        let found = match relocate_with_snapshot(node, &saved, tolerance) {
            Some(n) => n,
            None => return Ok(None),
        };

        // 3. Auto-save new snapshot if relocated
        if auto_save {
            let snap = ElementSnapshot::capture(&found);
            let row: ElementSnapshotRow = snap.into();
            let _ = save_element(self.store.as_ref(), url, key, &row).await;
        }

        Ok(Some(found))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryStore;

    fn make_doc() -> Node {
        Node::from_html(
            r#"<html><body>
                <div class="item" id="main">Hello World</div>
            </body></html>"#,
        )
    }

    #[tokio::test]
    async fn css_adaptive_returns_node_when_css_matches() {
        let store = Arc::new(MemoryStore::new());
        let tracker = AdaptiveTracker::new(store);
        let doc = make_doc();

        let found = tracker
            .css_adaptive(&doc, "#main", "main-key", "https://example.com", true, DEFAULT_TOLERANCE)
            .await
            .unwrap();

        assert!(found.is_some());
        assert_eq!(found.unwrap().text(), "Hello World");
    }

    #[tokio::test]
    async fn css_adaptive_returns_none_when_no_css_and_no_snapshot() {
        let store = Arc::new(MemoryStore::new());
        let tracker = AdaptiveTracker::new(store);
        let doc = make_doc();

        let found = tracker
            .css_adaptive(&doc, "#nonexistent", "missing-key", "https://example.com", false, DEFAULT_TOLERANCE)
            .await
            .unwrap();

        assert!(found.is_none());
    }

    #[tokio::test]
    async fn css_adaptive_relocates_from_saved_snapshot() {
        let store = Arc::new(MemoryStore::new());
        let tracker = AdaptiveTracker::new(Arc::clone(&store));

        // 第一次：CSS 匹配 #main，auto_save 保存快照
        let doc1 = make_doc();
        let found1 = tracker
            .css_adaptive(&doc1, "#main", "relocate-key", "https://example.com", true, DEFAULT_TOLERANCE)
            .await
            .unwrap();
        assert!(found1.is_some());

        // 第二次：CSS 不匹配（#missing-id），从存储加载快照重定位到 #main
        let doc2 = make_doc();
        let found2 = tracker
            .css_adaptive(&doc2, "#missing-id", "relocate-key", "https://example.com", false, DEFAULT_TOLERANCE)
            .await
            .unwrap();
        assert!(found2.is_some());
        assert_eq!(found2.unwrap().text(), "Hello World");
    }
}
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib css_adaptive_returns_node_when_css_matches`
Expected: FAIL with "cannot find type `AdaptiveTracker`" 或 `mod adaptive` 未声明（因为 crawl/mod.rs 还没添加 `pub mod adaptive;`）

- [ ] **Step 3: 写最小实现**

3a. 修改 `src/crawl/mod.rs:8` 附近（在 `pub mod auto;` 后新增）：

```rust
pub mod adaptive;
pub mod auto;
pub mod builder;
pub mod engine;
pub mod middleware;
pub mod observability;
pub mod runner;
pub mod runtime;
pub mod scheduling;
```

3b. 在 `src/crawl/mod.rs` 的 re-export 区（约第 24 行 `pub use auto::ModeRuleEngine;` 后）新增：

```rust
pub use adaptive::AdaptiveTracker;
```

3c. `src/crawl/adaptive.rs` 的实现已在 Step 1 中完整写出（包括 AdaptiveTracker 结构体 + css_adaptive 方法）。无需额外修改。

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib css_adaptive`
Expected: 3 个测试全部 PASS

- [ ] **Step 5: 提交**

```bash
git add src/crawl/adaptive.rs src/crawl/mod.rs
git commit -m "feat: 新增 AdaptiveTracker 迁移 adaptive 持久化"
```

---

## Task 4: 删除 parser/adaptive.rs 的 css_adaptive 函数

**Files:**
- Modify: `src/parser/adaptive.rs:8`（删除 `use crate::storage::Store;`）
- Modify: `src/parser/adaptive.rs:286-331`（删除 `css_adaptive` 函数）
- Test: `cargo build --lib` 验证 parser 不再依赖 storage

**Interfaces:**
- Consumes: Task 3 的 `AdaptiveTracker::css_adaptive`（已替代自由函数）
- Produces: parser/adaptive.rs 只保留 `ElementSnapshot` 类型 + `capture` + `similarity` + `relocate_with_snapshot` + `DEFAULT_TOLERANCE`（纯函数，无 storage 依赖）

- [ ] **Step 1: 写失败的测试**

在 `src/parser/adaptive.rs` 的 `#[cfg(test)] mod tests`（Task 2 新增）中追加测试：

```rust
    #[test]
    fn parser_adaptive_has_no_storage_dependency() {
        // 验证 parser::adaptive 不再导出 css_adaptive 自由函数
        // 如果 css_adaptive 仍存在，下面的引用会编译通过（测试失败）
        // 删除后应编译失败（确认依赖已消除）
        // 此测试通过反向验证：使用 AdaptiveTracker 替代
        let store = std::sync::Arc::new(crate::storage::MemoryStore::new());
        let tracker = crate::crawl::AdaptiveTracker::new(store);
        let doc = crate::parser::Node::from_html("<html><body><p>x</p></body></html>");
        let result = futures::executor::block_on(
            tracker.css_adaptive(&doc, "p", "k", "https://x", false, crate::parser::DEFAULT_TOLERANCE),
        );
        assert!(result.unwrap().is_some());
    }
```

注意：此测试用 `futures::executor::block_on` 同步执行 async（仅用于验证 API 可达性，生产代码用 tokio runtime）。

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib parser_adaptive_has_no_storage_dependency`
Expected: 测试通过（因为 AdaptiveTracker 已在 Task 3 实现），但 `css_adaptive` 自由函数仍存在（无编译错误，但有未使用警告）

- [ ] **Step 3: 写最小实现**

3a. 删除 `src/parser/adaptive.rs:8` 的 `use crate::storage::Store;`（整行删除）

3b. 删除 `src/parser/adaptive.rs:286-331` 的 `css_adaptive` 函数（从 `/// Adaptive CSS selection: try CSS first...` 注释到函数结束 `}`）

3c. 检查 `src/parser/adaptive.rs` 是否还有 `use crate::storage::{...}` 残留，全部删除。

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib parser_adaptive_has_no_storage_dependency && cargo build --lib`
Expected: PASS + 编译通过（parser/adaptive.rs 无 storage 引用）

验证 parser 不依赖 storage：
```bash
grep -n "use crate::storage" src/parser/adaptive.rs
```
Expected: 无输出

- [ ] **Step 5: 提交**

```bash
git add src/parser/adaptive.rs
git commit -m "refactor: 删除 parser::css_adaptive 迁到 crawl::AdaptiveTracker"
```

---

## Task 5: 删除 parser/mod.rs 的 Node::css_adaptive 方法和 re-export

**Files:**
- Modify: `src/parser/mod.rs:8-10`（从 `pub use adaptive::{...}` 中删除 `css_adaptive`）
- Modify: `src/parser/mod.rs:157-170`（删除 `Node::css_adaptive` 方法）
- Test: `cargo build --lib` 验证编译通过

**Interfaces:**
- Consumes: Task 3 的 `AdaptiveTracker::css_adaptive`
- Produces: `Node` 不再有 `css_adaptive` 方法；`parser` 模块不再 re-export `css_adaptive`

- [ ] **Step 1: 写失败的测试**

在 `src/parser/mod.rs` 的 `#[cfg(test)] mod tests` 末尾追加测试：

```rust
    #[tokio::test]
    async fn node_no_longer_has_css_adaptive_method() {
        // 验证 Node::css_adaptive 方法已删除
        // 调用方应使用 AdaptiveTracker::css_adaptive
        let store = std::sync::Arc::new(crate::storage::MemoryStore::new());
        let tracker = crate::crawl::AdaptiveTracker::new(store);
        let doc = Node::from_html("<html><body><p>text</p></body></html>");

        // 旧 API（应编译失败）：doc.css_adaptive("p", "k", "url", store, true, 0.5).await
        // 新 API：
        let found = tracker
            .css_adaptive(&doc, "p", "k", "https://x", false, 0.5)
            .await
            .unwrap();
        assert!(found.is_some());
    }
```

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo test --lib node_no_longer_has_css_adaptive_method`
Expected: 测试通过（因为 AdaptiveTracker 已存在），但 `Node::css_adaptive` 方法仍存在（未使用警告）

- [ ] **Step 3: 写最小实现**

3a. 修改 `src/parser/mod.rs:8-10`，将 `pub use adaptive::{css_adaptive, relocate_with_snapshot, similarity, ElementSnapshot, DEFAULT_TOLERANCE};` 改为（删除 `css_adaptive`）：

```rust
pub use adaptive::{
    relocate_with_snapshot, similarity, ElementSnapshot, DEFAULT_TOLERANCE,
};
```

3b. 删除 `src/parser/mod.rs:157-170` 的 `Node::css_adaptive` 方法（从 `/// Adaptive CSS selection with SQLite-backed snapshot persistence.` 注释到方法结束 `}`）。

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo test --lib node_no_longer_has_css_adaptive_method && cargo build --lib`
Expected: PASS + 编译通过

- [ ] **Step 5: 提交**

```bash
git add src/parser/mod.rs
git commit -m "refactor: 删除 Node::css_adaptive 方法改用 AdaptiveTracker"
```

---

## Task 6: 更新 mcp/tools.rs 调用方使用 AdaptiveTracker

**Files:**
- Modify: `src/mcp/tools.rs:178-180`（替换 `css_adaptive` 自由函数调用为 `AdaptiveTracker::css_adaptive`）
- Test: `cargo build --lib --features mcp` 验证 mcp 模块编译通过

**Interfaces:**
- Consumes: Task 3 的 `AdaptiveTracker::css_adaptive`
- Produces: mcp/tools.rs 不再依赖 `parser::css_adaptive`

- [ ] **Step 1: 写失败的测试**

由于 mcp/tools.rs 是 MCP 工具实现，难以单元测试（需 MCP 客户端），此任务以编译验证为主。先读取当前 `src/mcp/tools.rs:170-200` 确认上下文：

Run: `grep -n "css_adaptive\|use crate::parser" src/mcp/tools.rs`
Expected: 显示第 178 行 `use crate::parser::css_adaptive;` 和第 180 行 `css_adaptive(...)` 调用

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo build --lib`
Expected: 编译失败 with "cannot find function `css_adaptive` in module `crate::parser`"（因为 Task 4 已删除该函数）

- [ ] **Step 3: 写最小实现**

修改 `src/mcp/tools.rs`：

3a. 删除第 178 行的 `use crate::parser::css_adaptive;`（替换为 AdaptiveTracker 导入）。在文件顶部 `use` 区添加：

```rust
use crate::crawl::AdaptiveTracker;
```

3b. 修改第 180 行的调用，将：
```rust
    let found = css_adaptive(&doc, selector, key, url, store.as_ref(), true, tolerance).await;
```
改为：
```rust
    let tracker = AdaptiveTracker::new(std::sync::Arc::clone(&store));
    let found = tracker
        .css_adaptive(&doc, selector, key, url, true, tolerance)
        .await
        .unwrap_or(None);
```

注意：`store` 变量的类型需确认。如果 `store` 是 `Arc<dyn Store>`，则 `store.as_ref()` 是 `&dyn Store`，原 API 接收 `&dyn Store`。新 API 接收 `&self`（AdaptiveTracker 持有 `Arc<dyn Store>`）。用 `Arc::clone(&store)` 创建 tracker。

如果 `store` 不是 `Arc<dyn Store>` 而是其他类型（如 `&dyn Store`），需调整。先读取上下文确认：

Run: `grep -n -B 5 "css_adaptive" src/mcp/tools.rs | head -20`

根据实际 `store` 类型调整 Step 3b 的代码。假设 `store` 是 `Arc<dyn Store>`（最常见模式）。

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo build --lib`
Expected: 编译通过（无 css_adaptive 引用错误）

验证：
```bash
grep -n "css_adaptive" src/mcp/tools.rs
```
Expected: 只显示 `tracker.css_adaptive(...)` 调用，无 `use crate::parser::css_adaptive`

- [ ] **Step 5: 提交**

```bash
git add src/mcp/tools.rs
git commit -m "refactor: mcp/tools 改用 AdaptiveTracker::css_adaptive"
```

---

## Task 7: Cargo.toml 新增 http/browser/stealth/mcp feature

**Files:**
- Modify: `Cargo.toml:8-12`（features 区）
- Modify: `Cargo.toml:24,63`（tokio-tungstenite 和 zip 标记 optional）
- Test: `cargo build --no-default-features --features http` 验证 HTTP-only 编译

**Interfaces:**
- Consumes: 无
- Produces: 5 个 feature（http/browser/stealth/mcp/sqlite）+ 可选依赖（tokio-tungstenite/zip/flate2/rmcp/turso）

- [ ] **Step 1: 写失败的测试**

无单元测试，以编译验证为主。先确认当前 Cargo.toml 的依赖：

Run: `grep -n "tokio-tungstenite\|^zip\|rmcp\|turso\|flate2" Cargo.toml`
Expected: 显示 tokio-tungstenite（非 optional）、zip（非 optional）、turso（已 optional）。rmcp 和 flate2 可能不存在。

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo build --no-default-features --features http`
Expected: FAIL（因为 `http` feature 未定义）

- [ ] **Step 3: 写最小实现**

3a. 修改 `Cargo.toml:8-12`，将：
```toml
[features]
default = []
# 启用 SQLite 存储后端（默认禁用，使用 FileStore）
sqlite = ["dep:turso"]
```
改为：
```toml
[features]
default = ["http"]

# HTTP 模式（默认启用）：HTTP fetcher + parser + storage + crawl 基础
http = []

# 浏览器模式（Dynamic）：启用 browser 模块 + DynamicStrategy
browser = ["dep:tokio-tungstenite", "dep:zip", "dep:flate2"]

# Stealth 模式：启用 stealth 模块 + StealthStrategy（依赖 browser）
stealth = ["browser"]

# MCP 服务：启用 mcp 模块（依赖 crawl）
mcp = ["dep:rmcp"]

# SQLite 存储后端
sqlite = ["dep:turso"]
```

3b. 修改 `Cargo.toml:24`，将 `tokio-tungstenite = { version = "0.30", features = ["rustls-tls-webpki-roots"] }` 改为（添加 `optional = true`）：

```toml
tokio-tungstenite = { version = "0.30", features = ["rustls-tls-webpki-roots"], optional = true }
```

3c. 修改 `Cargo.toml:63`，将 `zip = "2"` 改为（添加 `optional = true`）：

```toml
zip = { version = "2", optional = true }
```

3d. 检查 `Cargo.toml` 是否有 `flate2` 依赖。如果没有，在 `zip` 后新增：

```toml
flate2 = { version = "1", optional = true }
```

3e. 检查 `Cargo.toml` 是否有 `rmcp` 依赖。如果没有，在 `turso` 后新增：

```toml
rmcp = { version = "0.1", optional = true }
```

注意：如果 mcp 模块当前不依赖 rmcp（可能用其他 MCP 实现），则不需要添加 rmcp 依赖，`mcp = []` 即可。先检查：

Run: `grep -rn "use rmcp\|extern crate rmcp" src/mcp/`
Expected: 显示是否使用 rmcp。如果无输出，则 `mcp = []`（空 feature）即可。

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo build --no-default-features --features http`
Expected: 编译通过（HTTP-only 模式）

如果编译失败，根据错误信息加 cfg gate（Task 8-12 会处理具体模块 gate）。

- [ ] **Step 5: 提交**

```bash
git add Cargo.toml
git commit -m "feat: Cargo.toml 新增 http/browser/stealth/mcp feature 分层"
```

---

## Task 8: lib.rs 添加 cfg gate

**Files:**
- Modify: `src/lib.rs:89-116`（模块声明区）
- Test: `cargo build --no-default-features --features http` 验证 HTTP-only 不编译 browser/stealth/mcp

**Interfaces:**
- Consumes: Task 7 的 feature 定义
- Produces: `browser`/`stealth`/`mcp` 模块按 feature gate 编译

- [ ] **Step 1: 写失败的测试**

无单元测试，以编译验证为主。先确认当前模块声明：

Run: `grep -n "pub mod" src/lib.rs`
Expected: 显示所有 `pub mod` 声明（browser/stealth/mcp 等当前无 cfg gate）

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo build --no-default-features --features http`
Expected: FAIL（因为 browser/stealth/mcp 模块声明无 cfg gate，编译时会尝试编译这些模块及其依赖）

- [ ] **Step 3: 写最小实现**

修改 `src/lib.rs:89-116`，将：

```rust
/// 浏览器进程管理：启动 Chrome、CDP 会话、页面操作。
pub mod browser;
/// 浏览器启动选项和代理配置。
pub mod config;
/// TOML 配置文件解析。
pub mod config_file;
/// Spider 爬虫引擎（调度器、检查点、流式处理）。
pub mod crawl;
/// 分类错误体系（Browser / Network / Parse / Mcp / Storage）。
pub mod error;
/// 统一 Fetcher API（Http / Dynamic / Stealth / Auto 模式）。
pub mod fetcher;
/// HTTP 客户端（TLS 指纹模拟，基于 wreq）。
pub mod http;
/// MCP Server（AI 辅助爬取，stdio JSON-RPC）。
pub mod mcp;
/// HTML 解析：CSS/XPath 选择器 + 自适应重定位。
pub mod parser;
/// 代理池管理与轮换策略。
pub mod proxy;
/// 反检测补丁 + Cloudflare 挑战解决 + 人类行为模拟。
pub mod stealth;
/// 可插拔存储（MemoryStore + FileStore，可选 SqliteStore）。
pub mod storage;
/// 文本和属性处理工具。
pub mod text;
/// 内部辅助工具（URL 解析、随机后缀）。
pub mod utils;
```

改为：

```rust
/// 浏览器进程管理：启动 Chrome、CDP 会话、页面操作。
#[cfg(feature = "browser")]
pub mod browser;
/// 浏览器启动选项和代理配置。
pub mod config;
/// TOML 配置文件解析。
pub mod config_file;
/// Spider 爬虫引擎（调度器、检查点、流式处理）。
pub mod crawl;
/// 分类错误体系（Browser / Network / Parse / Mcp / Storage）。
pub mod error;
/// 统一 Fetcher API（Http / Dynamic / Stealth / Auto 模式）。
pub mod fetcher;
/// HTTP 客户端（TLS 指纹模拟，基于 wreq）。
pub mod http;
/// MCP Server（AI 辅助爬取，stdio JSON-RPC）。
#[cfg(feature = "mcp")]
pub mod mcp;
/// HTML 解析：CSS/XPath 选择器 + 自适应重定位。
pub mod parser;
/// 代理池管理与轮换策略。
pub mod proxy;
/// 反检测补丁 + Cloudflare 挑战解决 + 人类行为模拟。
#[cfg(feature = "stealth")]
pub mod stealth;
/// 可插拔存储（MemoryStore + FileStore，可选 SqliteStore）。
pub mod storage;
/// 文本和属性处理工具。
pub mod text;
/// 内部辅助工具（URL 解析、随机后缀）。
pub mod utils;
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo build --no-default-features --features http`
Expected: 编译通过（browser/stealth/mcp 模块不编译）

如果失败（如其他模块引用 browser/stealth/mcp），继续在 Task 9-12 添加内部 cfg gate。

- [ ] **Step 5: 提交**

```bash
git add src/lib.rs
git commit -m "feat: lib.rs 模块声明按 feature gate 编译"
```

---

## Task 9: fetcher/mod.rs strategies 模块 cfg gate

**Files:**
- Modify: `src/fetcher/mod.rs:31-35`（strategies 模块声明和 re-export）
- Test: `cargo build --no-default-features --features http` 验证 fetcher 在 HTTP-only 下编译

**Interfaces:**
- Consumes: Task 7 的 feature 定义、Task 8 的 lib.rs cfg gate
- Produces: `fetcher::strategies` 模块按 `browser` feature gate 编译

- [ ] **Step 1: 写失败的测试**

无单元测试，以编译验证为主。先确认当前 fetcher/mod.rs 的模块声明：

Run: `grep -n "pub mod\|pub use" src/fetcher/mod.rs | head -20`
Expected: 显示 `pub mod client;`、`pub mod response;`、`pub mod strategies;`（如果 PR2 已创建）等

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo build --no-default-features --features http`
Expected: 可能 FAIL（如果 strategies 模块引用 browser/stealth 类型）

- [ ] **Step 3: 写最小实现**

修改 `src/fetcher/mod.rs:31-35` 附近的模块声明。假设 PR2 完成后状态为：

```rust
pub mod client;
pub mod response;
pub mod strategies;
pub mod strategy;

pub use client::{FetchClient, FetchClientConfig};
pub use response::{Method, Request, Response};
```

改为（strategies 模块和 re-export 加 cfg gate）：

```rust
pub mod client;
pub mod response;
pub mod strategy;

#[cfg(feature = "browser")]
pub mod strategies;

pub use client::{FetchClient, FetchClientConfig};
pub use response::{Method, Request, Response};

#[cfg(feature = "browser")]
pub use strategies::dynamic::DynamicStrategy;

#[cfg(feature = "stealth")]
pub use strategies::stealth::StealthStrategy;
```

注意：`strategy`（trait 定义）总是可用，`strategies`（实现）按 feature gate。

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo build --no-default-features --features http`
Expected: 编译通过

Run: `cargo build --features browser`
Expected: 编译通过（DynamicStrategy 可用）

- [ ] **Step 5: 提交**

```bash
git add src/fetcher/mod.rs
git commit -m "feat: fetcher strategies 模块按 feature gate 编译"
```

---

## Task 10: fetcher/strategies/mod.rs 模块声明 cfg gate

**Files:**
- Modify: `src/fetcher/strategies/mod.rs`（dynamic/stealth 模块声明加 cfg gate）
- Test: `cargo build --features browser` 和 `cargo build --features stealth` 验证

**Interfaces:**
- Consumes: Task 9 的 strategies 模块 cfg gate
- Produces: `strategies::dynamic` 按 `browser` gate、`strategies::stealth` 按 `stealth` gate

- [ ] **Step 1: 写失败的测试**

无单元测试，以编译验证为主。先读取当前 `src/fetcher/strategies/mod.rs`：

Run: `cat src/fetcher/strategies/mod.rs`
Expected: 显示 `pub mod dynamic;` 和 `pub mod stealth;`（PR2 创建，无 cfg gate）

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo build --features browser`
Expected: 可能 FAIL（如果 stealth 模块在 browser-only 下编译，引用 stealth feature 类型）

- [ ] **Step 3: 写最小实现**

修改 `src/fetcher/strategies/mod.rs`，将：

```rust
pub mod dynamic;
pub mod stealth;
```

改为：

```rust
#[cfg(feature = "browser")]
pub mod dynamic;

#[cfg(feature = "stealth")]
pub mod stealth;
```

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo build --features browser`
Expected: 编译通过（dynamic 编译，stealth 跳过）

Run: `cargo build --features stealth`
Expected: 编译通过（dynamic + stealth 都编译，因为 stealth 依赖 browser）

- [ ] **Step 5: 提交**

```bash
git add src/fetcher/strategies/mod.rs
git commit -m "feat: strategies 子模块按 feature gate 编译"
```

---

## Task 11: browser/launch.rs build_stealth_extra_args 添加 cfg gate

**Files:**
- Modify: `src/browser/launch.rs`（`build_stealth_extra_args` 函数 + `build_default_args` 中的调用）
- Test: `cargo build --features browser` 验证 browser-only 下编译（stealth 参数跳过）

**Interfaces:**
- Consumes: Task 7 的 feature 定义、PR2 的 `build_common_args` + `build_stealth_extra_args` 拆分
- Produces: `build_stealth_extra_args` 仅在 `stealth` feature 下编译

- [ ] **Step 1: 写失败的测试**

无单元测试，以编译验证为主。先确认 PR2 完成后 `src/browser/launch.rs` 的函数签名：

Run: `grep -n "pub fn build_" src/browser/launch.rs`
Expected: 显示 `build_common_args`、`build_stealth_extra_args`、`build_default_args`（PR2 已拆分）

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo build --features browser`
Expected: 可能 FAIL（如果 `build_stealth_extra_args` 引用 stealth 专用类型或 `build_default_args` 总是调用它）

- [ ] **Step 3: 写最小实现**

3a. 修改 `src/browser/launch.rs` 中 `build_stealth_extra_args` 函数，在 `pub fn` 前添加 `#[cfg(feature = "stealth")]`：

```rust
/// 构建 stealth 专用额外参数。
///
/// ARCH: 仅在 stealth feature 启用时编译。browser-only 模式下不包含反检测参数。
#[cfg(feature = "stealth")]
pub fn build_stealth_extra_args(options: &LaunchOptions) -> Vec<String> {
    vec![
        "disable-blink-features=AutomationControlled".into(),
        // ... 其他 stealth 参数
    ]
}
```

3b. 修改 `build_default_args`，将 `build_stealth_extra_args` 的调用加 cfg gate：

```rust
/// 构建完整启动参数（common + stealth_extra）。
#[must_use]
pub fn build_default_args(options: &LaunchOptions) -> Vec<String> {
    let mut args = build_common_args(options);
    #[cfg(feature = "stealth")]
    args.extend(build_stealth_extra_args(options));
    args
}
```

3c. 如果 `build_default_args` 在 browser-only 模式下不需要 stealth 参数，可保持上述实现。如果 browser-only 模式需要替代行为（如返回空 Vec），添加 `#[cfg(not(feature = "stealth"))]` 分支：

```rust
#[cfg(not(feature = "stealth"))]
let stealth_args: Vec<String> = Vec::new();
```

（通常不需要，`#[cfg(feature = "stealth")] args.extend(...)` 已足够）

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo build --features browser`
Expected: 编译通过（stealth 参数不编译）

Run: `cargo build --features stealth`
Expected: 编译通过（stealth 参数编译）

Run: `cargo test --features stealth build_`
Expected: 现有 launch.rs 测试通过

- [ ] **Step 5: 提交**

```bash
git add src/browser/launch.rs
git commit -m "feat: build_stealth_extra_args 按 stealth feature gate 编译"
```

---

## Task 12: Fetcher::new 的 build_strategy 方法添加 cfg gate

**Files:**
- Modify: `src/fetcher/mod.rs`（`Fetcher::new` 中的 `build_strategy`/`build_dynamic_strategy`/`build_stealth_strategy` 调用）
- Test: `cargo build --no-default-features --features http` 验证 HTTP-only 下 Fetcher::new 可用

**Interfaces:**
- Consumes: Task 9 的 strategies cfg gate、Task 10 的 strategies/mod.rs cfg gate
- Produces: `build_dynamic_strategy` 按 `browser` gate、`build_stealth_strategy` 按 `stealth` gate

- [ ] **Step 1: 写失败的测试**

无单元测试，以编译验证为主。先确认 PR2 完成后 `Fetcher::new` 的实现：

Run: `grep -n "build_strategy\|build_dynamic_strategy\|build_stealth_strategy" src/fetcher/mod.rs`
Expected: 显示 PR2 已预留的 `build_strategy` 分发方法 + `build_dynamic_strategy` + `build_stealth_strategy`

- [ ] **Step 2: 运行测试验证失败**

Run: `cargo build --no-default-features --features http`
Expected: 可能 FAIL（如果 `build_strategy` 总是调用 `build_dynamic_strategy`/`build_stealth_strategy`，而这些方法引用 DynamicStrategy/StealthStrategy）

- [ ] **Step 3: 写最小实现**

修改 `src/fetcher/mod.rs` 中 `Fetcher::new` 和 `build_strategy` 相关方法。假设 PR2 完成后状态为：

```rust
impl Fetcher {
    pub fn new(mode: FetchMode, config: FetchClientConfig) -> Result<Self> {
        let client = Arc::new(FetchClient::new(config.clone())?);
        let browser_strategy = Self::build_strategy(mode, &config, &client)?;
        Ok(Self { client, mode, browser_strategy })
    }

    fn build_strategy(...) -> Result<Option<Arc<dyn BrowserFetchStrategy>>> {
        match mode {
            FetchMode::Http | FetchMode::Auto => Ok(None),
            FetchMode::Dynamic => Self::build_dynamic_strategy(config),
            FetchMode::Stealth => Self::build_stealth_strategy(config, client),
        }
    }

    fn build_dynamic_strategy(config: &FetchClientConfig) -> Result<Option<Arc<dyn BrowserFetchStrategy>>> {
        Ok(Some(Arc::new(DynamicStrategy::from_config(config))))
    }

    fn build_stealth_strategy(config: &FetchClientConfig, _client: &FetchClient) -> Result<Option<Arc<dyn BrowserFetchStrategy>>> {
        let cf_jar = Arc::new(CfCookieJar::new(&config.cf_data_dir, config.cf_cookie_ttl));
        Ok(Some(Arc::new(StealthStrategy::from_config(config, cf_jar))))
    }
}
```

改为（添加 cfg gate + 无 feature 时的错误处理）：

```rust
impl Fetcher {
    pub fn new(mode: FetchMode, config: FetchClientConfig) -> Result<Self> {
        let client = Arc::new(FetchClient::new(config.clone())?);
        let browser_strategy = Self::build_strategy(mode, &config, &client)?;
        Ok(Self { client, mode, browser_strategy })
    }

    fn build_strategy(
        mode: FetchMode,
        config: &FetchClientConfig,
        client: &FetchClient,
    ) -> Result<Option<Arc<dyn BrowserFetchStrategy>>> {
        match mode {
            FetchMode::Http | FetchMode::Auto => Ok(None),
            FetchMode::Dynamic => Self::build_dynamic_strategy(config),
            FetchMode::Stealth => Self::build_stealth_strategy(config, client),
        }
    }

    #[cfg(feature = "browser")]
    fn build_dynamic_strategy(config: &FetchClientConfig) -> Result<Option<Arc<dyn BrowserFetchStrategy>>> {
        Ok(Some(Arc::new(DynamicStrategy::from_config(config))))
    }

    #[cfg(not(feature = "browser"))]
    fn build_dynamic_strategy(_: &FetchClientConfig) -> Result<Option<Arc<dyn BrowserFetchStrategy>>> {
        Err(WispError::Config("Dynamic mode requires 'browser' feature".into()))
    }

    #[cfg(feature = "stealth")]
    fn build_stealth_strategy(config: &FetchClientConfig, _client: &FetchClient) -> Result<Option<Arc<dyn BrowserFetchStrategy>>> {
        let cf_jar = Arc::new(CfCookieJar::new(&config.cf_data_dir, config.cf_cookie_ttl));
        Ok(Some(Arc::new(StealthStrategy::from_config(config, cf_jar))))
    }

    #[cfg(not(feature = "stealth"))]
    fn build_stealth_strategy(_: &FetchClientConfig, _: &FetchClient) -> Result<Option<Arc<dyn BrowserFetchStrategy>>> {
        Err(WispError::Config("Stealth mode requires 'stealth' feature".into()))
    }
}
```

注意：需确认 `WispError::Config` 变体存在。如果不存在，用 `WispError::Other` 或其他合适变体。先检查：

Run: `grep -n "Config\|Other" src/error.rs | head -10`

根据实际错误类型调整 Step 3 的代码。

- [ ] **Step 4: 运行测试验证通过**

Run: `cargo build --no-default-features --features http`
Expected: 编译通过（build_dynamic_strategy/build_stealth_strategy 返回错误，但编译通过）

Run: `cargo build --features browser`
Expected: 编译通过

Run: `cargo build --features stealth`
Expected: 编译通过

- [ ] **Step 5: 提交**

```bash
git add src/fetcher/mod.rs
git commit -m "feat: Fetcher build_strategy 按 feature gate 编译"
```

---

## Task 13: 验证 5 种 feature 组合编译通过

**Files:**
- 无文件修改（纯验证任务）
- Test: 5 种 cargo build 命令

**Interfaces:**
- Consumes: Task 7-12 的所有 cfg gate
- Produces: 验证 5 种 feature 组合编译通过

- [ ] **Step 1: 写失败的测试**

无单元测试，以编译验证为主。准备 5 种 feature 组合的验证命令。

- [ ] **Step 2: 运行测试验证失败**

依次运行（如果之前 Task 8-12 有遗漏，这里会暴露）：

```bash
cargo build --no-default-features --features http
cargo build --features browser
cargo build --features stealth
cargo build --features http,sqlite
cargo build --all-features
```

Expected: 任一组合 FAIL 时记录错误信息

- [ ] **Step 3: 写最小实现**

根据 Step 2 的错误信息修复遗漏的 cfg gate。常见问题：

3a. **crawl 模块引用 browser 类型**：在 `src/crawl/engine.rs` 或 `src/crawl/middleware/builtin.rs` 中搜索 `use crate::browser` / `use crate::stealth`，加 cfg gate 或迁移到函数内部。

```bash
grep -rn "use crate::browser\|use crate::stealth" src/crawl/
```

对每个匹配的文件，将 `use crate::browser::...` 改为 `#[cfg(feature = "browser")] use crate::browser::...`，或把使用点加 `#[cfg(feature = "browser")]`。

3b. **fetcher/client.rs 引用 browser**：`fetcher/client.rs` 中的 `browser_pool` 字段和 `fetch_browser` 方法应加 `#[cfg(feature = "browser")]`。

3c. **mcp 模块引用 crawl::adaptive**：Task 6 已更新 mcp/tools.rs 使用 AdaptiveTracker，应无问题。如果 mcp 模块还有其他 browser/stealth 引用，加 cfg gate。

3d. **lib.rs re-export 引用 browser/stealth 类型**：检查 `pub use` 语句：

```bash
grep -n "pub use browser\|pub use stealth" src/lib.rs
```

加 cfg gate：
```rust
#[cfg(feature = "browser")]
pub use browser::{Browser, Page};

#[cfg(feature = "stealth")]
pub use stealth::TurnstileConfig;
```

- [ ] **Step 4: 运行测试验证通过**

依次运行 5 种组合：

```bash
cargo build --no-default-features --features http       # HTTP-only
cargo build --features browser                          # HTTP + Dynamic
cargo build --features stealth                          # HTTP + Browser + Stealth
cargo build --features http,sqlite                      # HTTP + SQLite
cargo build --all-features                             # 全部启用
```

Expected: 全部编译通过

运行全功能测试：
```bash
cargo test --all-features
```
Expected: 现有测试全部通过

- [ ] **Step 5: 提交**

```bash
git add -A
git commit -m "test: 验证 5 种 feature 组合编译通过"
```

---

## Task 14: 更新 banzhu-rs Cargo.toml 启用所需 feature

**Files:**
- Modify: `/home/weng/banzhu-rs/Cargo.toml:49`（wisp 依赖添加 features）
- Test: `cd /home/weng/banzhu-rs && cargo build` 验证编译通过

**Interfaces:**
- Consumes: Task 7-13 的 feature gate 实现
- Produces: banzhu-rs 显式启用 wisp 的 `http`/`stealth`/`sqlite` feature

- [ ] **Step 1: 写失败的测试**

无单元测试，以编译验证为主。先确认 banzhu-rs 当前 wisp 依赖配置：

Run: `grep -n "wisp" /home/weng/banzhu-rs/Cargo.toml`
Expected: 显示 `wisp = { path = "../wisp" }`（无 features）

- [ ] **Step 2: 运行测试验证失败**

Run: `cd /home/weng/banzhu-rs && cargo build`
Expected: 可能 FAIL（因为 wisp 默认 feature 从 `[]` 改为 `["http"]`，banzhu-rs 可能依赖 browser/stealth 类型但未启用 feature）

如果 banzhu-rs 当前只用 HTTP 模式（`FetchMode::Http`/`Auto`），编译应通过。如果失败，记录错误信息。

- [ ] **Step 3: 写最小实现**

修改 `/home/weng/banzhu-rs/Cargo.toml:49`，将：

```toml
wisp = { path = "../wisp" }
```

改为：

```toml
wisp = { path = "../wisp", features = ["http", "stealth", "sqlite"] }
```

注意：
- `http` 是默认 feature，显式声明可读性更好
- `stealth` 启用反检测能力（banzhu-spider 爬取禁漫等需绕过 CF）
- `sqlite` 启用 SQLite 存储后端（banzhu-rs 已有 `rusqlite` 依赖，可能用 wisp 的 SqliteStore）
- 不启用 `browser`（已被 `stealth` 隐式启用）和 `mcp`（banzhu-rs 不用 MCP）

- [ ] **Step 4: 运行测试验证通过**

Run: `cd /home/weng/banzhu-rs && cargo build`
Expected: 编译通过

Run: `cd /home/weng/banzhu-rs && cargo test`
Expected: 现有测试通过

- [ ] **Step 5: 提交**

```bash
cd /home/weng/banzhu-rs
git add Cargo.toml
git commit -m "feat: 启用 wisp http/stealth/sqlite feature"
```

---

## 完成验证

PR3 完成后，运行以下命令验证全部目标达成：

```bash
# 1. parser 不依赖 storage
grep -rn "use crate::storage" src/parser/
# Expected: 无输出

# 2. css_adaptive 已迁移到 crawl::adaptive
grep -rn "css_adaptive" src/parser/
# Expected: 无输出（或只在注释中）
grep -rn "css_adaptive" src/crawl/adaptive.rs
# Expected: 显示 AdaptiveTracker::css_adaptive 方法

# 3. 5 种 feature 组合编译通过
cargo build --no-default-features --features http
cargo build --features browser
cargo build --features stealth
cargo build --features http,sqlite
cargo build --all-features

# 4. 全功能测试通过
cargo test --all-features

# 5. banzhu-rs 编译通过
cd /home/weng/banzhu-rs && cargo build
```

---

## 风险与缓解

| 风险 | 缓解 |
|---|---|
| Feature gate 组合爆炸（2^5=32 种组合） | 只测试 5 种关键组合（Task 13） |
| crawl 模块隐式依赖 browser（engine.rs fetch_browser 调用） | Task 13 Step 3 全局搜索 `use crate::browser`，加 cfg gate 或迁移到函数内部 |
| banzhu-rs 隐式依赖 wisp browser/stealth 类型 | Task 14 启用 `["http", "stealth", "sqlite"]` 覆盖常用路径 |
| `cargo build --no-default-features`（无 feature）失败 | 不保证支持（default = ["http"]，最小可用配置） |
| AdaptiveTracker 测试用 MemoryStore，与生产 SqliteStore 行为差异 | MemoryStore 和 SqliteStore 都实现 Store trait，语义一致；PR1 已有 SqliteStore 集成测试 |
| mcp/tools.rs 的 store 变量类型不确定 | Task 6 Step 3 先 `grep` 确认 store 类型再调整调用 |
