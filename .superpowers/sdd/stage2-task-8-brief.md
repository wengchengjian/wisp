# Task 8: 端到端集成测试与 stage 2 完成验证

**Files:**
- Modify: `tests/integration.rs`（在 `mod adaptive_test { ... }` 末尾追加 3 个测试）

**目标：** 验证 stage 2 的三大改动（Node 重构 + DOM 导航 + XPath 集成 + adaptive capture 升级）在端到端场景下协同工作。

**API 修正说明（plan 原文有误，以本 brief 为准）：**
1. `ElementSnapshot` 未实现 `From<ElementSnapshotRow>`，必须用 `ElementSnapshot::from_row(row)` 而非 `row.into()`
2. `Node::css_adaptive` 是 `&self` 方法（委托到 `adaptive::css_adaptive(self, ...)`），调用方式 `doc.css_adaptive(...)` 正确
3. `Node::select(&self, css: &str) -> NodeList`，`NodeList::len()` 可用

## Step 1: 读取 tests/integration.rs 当前结构

读取 `tests/integration.rs` 的 `mod adaptive_test { ... }`（约 line 117-160），确认现有 `test_end_to_end_adaptive_relocation` 测试的位置和风格。

## Step 2: 在 mod adaptive_test 末尾追加 3 个测试

在 `tests/integration.rs` 的 `mod adaptive_test { ... }` 闭合大括号 `}` 之前（即 `test_end_to_end_adaptive_relocation` 之后），追加以下 3 个测试：

```rust
    #[test]
    fn test_dom_navigation_with_adaptive_snapshot() {
        // 验证 Node 重构后 adaptive 仍正常工作，且 capture 用了导航 API
        let store = Store::open_in_memory().unwrap();
        let url = "https://shop.example.com/products";

        let html = r#"
        <html><body>
          <div class="products">
            <div class="product" data-id="1">
              <h3 class="title">Widget</h3>
            </div>
          </div>
        </body></html>
        "#;

        let doc = Node::from_html(html);
        let node = doc.css_adaptive(".title", "product-title", url, &store, true, 0.5);
        assert!(node.is_some());
        assert_eq!(node.unwrap().text(), "Widget");

        // 验证 capture 用了导航 API：检查 snapshot 的 ancestor_path 包含 "div.products"
        let saved = store.load_element(url, "product-title").unwrap().expect("snapshot should be saved");
        let snapshot = wisp::parser::ElementSnapshot::from_row(saved);
        assert!(snapshot.ancestor_path.iter().any(|p| p.contains("products")));
    }

    #[test]
    fn test_xpath_and_css_consistency() {
        // 验证 XPath 和 CSS 对同一查询返回一致结果
        let html = r#"
        <html><body>
          <ul>
            <li class="item">A</li>
            <li class="item">B</li>
            <li class="item">C</li>
          </ul>
        </body></html>
        "#;

        let doc = Node::from_html(html);
        let css_result = doc.select("li.item");
        let xpath_result = doc.xpath("//li[@class='item']");

        assert_eq!(css_result.len(), xpath_result.len());
        assert_eq!(css_result.len(), 3);
    }

    #[test]
    fn test_node_shares_document_after_select() {
        // 验证 select 返回的 Node 共享同一 Document（导航可工作）
        let html = r#"<html><body><div><p>Hello</p></div></body></html>"#;
        let doc = Node::from_html(html);
        let p = doc.select_one("p").expect("p should exist");
        // 阶段 1 的 fragment 模型下 parent() 返回 None
        // 阶段 2 重构后 parent() 应返回 div
        let parent = p.parent().expect("parent should work after refactor");
        assert_eq!(parent.tag(), "div");
    }
```

**注意事项：**
- `mod adaptive_test` 顶部已有 `use wisp::parser::Node;` 和 `use wisp::storage::Store;`，新测试无需重复 use
- `wisp::parser::ElementSnapshot` 用全路径调用（避免在 mod 顶部加 use 影响其他测试）
- 保持现有缩进（4 空格，与 `test_end_to_end_adaptive_relocation` 一致）

## Step 3: 运行新测试

Run: `cargo test --test integration adaptive_test`
Expected: 4 passed（1 原有 + 3 新增）

## Step 4: 运行完整测试套件

Run: `cargo test --lib && cargo test --test adaptive_test && cargo test --test crawl_checkpoint_test && cargo test --test difflib_test && cargo test --test dom_navigation_test && cargo test --test xpath_test && cargo test --test integration`
Expected: 全部通过（lib 35 + adaptive 5 + checkpoint 4 + difflib 7 + dom_nav 9 + xpath 9 + integration 4 = 73 passed）

## Step 5: 提交

```bash
git add tests/integration.rs
git commit -m "test: 阶段 2 端到端集成测试（DOM 导航 + XPath + adaptive 一致性）"
```

## 全局约束
- Rust 2021 edition，wisp 项目位于 `f:\project\wisp`
- 最小改动，不碰 src/ 代码
- PowerShell 无 heredoc，git 提交用单个 -m
- Commit message 中文
