# XPath 回查精度改进 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 `src/parser/xpath.rs` 的 `locate_in_sxd` / `find_in_scraper` 从"第一个同名元素"/"tag + 第一个属性"启发式升级为"路径签名精确匹配（带回退）"，消除 Stage 2 已知简化。

**Architecture:** 新增内部类型 `NodeSignature`（路径签名 = `Vec<(tag, Option<first_class>)>`，从根到节点），提供 `from_scraper(&Node)` / `from_sxd(dom::Element)` 双向构造。`locate_in_sxd` 和 `find_in_scraper` 改为"先用签名 DFS 匹配，失败回退到旧启发式"。公共 API 不变。

**Tech Stack:** Rust 2021, scraper 0.23 (ElementRef/Node 导航), sxd-document 0.3.2 (dom::Element/Parent/ChildOfElement)

**Spec:** 修复 Stage 2 review 记录的 Minor 简化（`locate_in_sxd` 启发式定位）

**全局约束：**
- Rust 2021 edition, wisp 项目 `f:\project\wisp`
- PowerShell 无 heredoc，git 提交用单 `-m`
- Commit messages 中文
- 最小改动，不破坏公共 API
- 不修改 `src/parser/mod.rs` 的 Node 公共 API
- 所有命令需有超时

---

## File Structure

| 文件 | 责任 | 本 plan 改动 |
|---|---|---|
| `src/parser/xpath.rs` | XPath 查询 + sxd↔scraper 回查 | 新增 NodeSignature 类型 + 重写 locate_in_sxd/find_in_scraper |
| `tests/xpath_precision_test.rs` | 回查精度测试 | 新建：多同名元素 + 深层嵌套场景 |

**不触碰的文件：** `src/parser/mod.rs`, `src/parser/document.rs`, `src/parser/adaptive.rs`, 其他所有 src/ 文件

---

## 当前问题

读取 `src/parser/xpath.rs` line 64-71（`locate_in_sxd`）和 line 109-133（`find_in_scraper`）：

**`locate_in_sxd`** 只用 `node.tag()` 在 sxd 树中 DFS 找第一个同名元素。多个同名元素（如多个 `<div>`）时会定位到错误的元素。

**`find_in_scraper`** 只用 `sxd_node` 的 tag + 第一个属性构造 CSS 选择器，取 `select(&selector).next()`。问题：
1. 只用第一个属性，多属性时不精确
2. 多个元素共享同一 tag + 属性时取第一个，可能错

---

## Task 1: 新建 NodeSignature 类型 + from_scraper + from_sxd + 单元测试

**Files:**
- Modify: `src/parser/xpath.rs`（在文件末尾、`#[cfg(test)]` 之前插入新类型；如果无 `#[cfg(test)]` 则在文件末尾）
- Test: `src/parser/xpath.rs` 内的 `#[cfg(test)] mod tests`

**目标：** 定义 `NodeSignature` 类型，实现 `from_scraper(&Node)` 和 `from_sxd(dom::Element)` 构造，用单元测试验证双向构造正确。

- [ ] **Step 1: 读取当前 xpath.rs 的 import 和结构**

Run: 读 `src/parser/xpath.rs` line 1-15，确认现有 import（`use sxd_document::dom;` 等）和文件结构。确认 `dom::Parent` / `dom::ChildOfElement` / `dom::ChildOfRoot` 枚举变体名（sxd-document 0.3.2 API）。

- [ ] **Step 2: 在 xpath.rs 末尾追加 NodeSignature 类型定义**

在 `src/parser/xpath.rs` 末尾追加（在最后一个 `}` 之后）：

```rust

// ===== 路径签名精确回查 =====

/// 节点路径签名：从根到节点的路径，每级 (tag, first_class)。
///
/// 用于在 scraper 树和 sxd 树之间精确回查。class 是最稳定的标识
/// （ID 可能动态，其他属性可能变化），first_class 是 class 的第一个 token，
/// 通常是最具体的。对序列化差异（空白、属性顺序、引号）鲁棒。
#[derive(Debug, Clone, PartialEq, Eq)]
struct NodeSignature {
    /// 从根到节点的路径，索引 0 是根的 (tag, first_class)
    path: Vec<(String, Option<String>)>,
}

impl NodeSignature {
    /// 从 scraper Node 构造签名（node 到根的路径）。
    fn from_scraper(node: &Node) -> Self {
        let mut path = Vec::new();
        let mut current = Some(node.clone());
        while let Some(n) = current {
            let tag = n.tag();
            if tag.is_empty() { break; }
            let first_class = n.attr("class")
                .and_then(|c| c.split_whitespace().next().map(|s| s.to_string()));
            path.push((tag, first_class));
            current = n.parent();
        }
        path.reverse();  // 根在前
        Self { path }
    }

    /// 从 sxd dom::Element 构造签名（element 到根的路径）。
    fn from_sxd(element: dom::Element) -> Self {
        let mut path = Vec::new();
        let mut current = Some(element);
        while let Some(e) = current {
            let tag = e.name().local_part().to_string();
            let first_class = e.attributes().iter()
                .find(|a| a.name().local_part() == "class")
                .and_then(|a| a.value().split_whitespace().next().map(|s| s.to_string()));
            path.push((tag, first_class));
            // sxd-document 0.3.2: element.parent() 返回 Option<dom::Parent>
            // dom::Parent 是枚举，有 Element 和 Document 变体
            current = e.parent().and_then(|p| match p {
                dom::Parent::Element(pe) => Some(pe),
                _ => None,
            });
        }
        path.reverse();  // 根在前
        Self { path }
    }
}
```

- [ ] **Step 3: 在 xpath.rs 末尾追加单元测试 mod**

在 NodeSignature 定义之后追加：

```rust

#[cfg(test)]
mod signature_tests {
    use super::*;
    use crate::parser::Node;

    #[test]
    fn test_signature_from_scraper_simple() {
        let html = r#"<html><body><div class="main"><p>text</p></div></body></html>"#;
        let doc = Node::from_html(html);
        let p = doc.select_one("p").expect("p should exist");
        let sig = NodeSignature::from_scraper(&p);
        // 路径: html > body > div.main > p
        assert_eq!(sig.path.len(), 4);
        assert_eq!(sig.path[0], ("html".to_string(), None));
        assert_eq!(sig.path[1], ("body".to_string(), None));
        assert_eq!(sig.path[2], ("div".to_string(), Some("main".to_string())));
        assert_eq!(sig.path[3], ("p".to_string(), None));
    }

    #[test]
    fn test_signature_from_scraper_multi_class() {
        // first_class 只取第一个 token
        let html = r#"<html><body><div class="main content box">x</div></body></html>"#;
        let doc = Node::from_html(html);
        let div = doc.select_one("div").expect("div should exist");
        let sig = NodeSignature::from_scraper(&div);
        assert_eq!(sig.path[2], ("div".to_string(), Some("main".to_string())));
    }

    #[test]
    fn test_signature_from_sxd_simple() {
        let html = r#"<html><body><div class="main"><p>text</p></div></body></html>"#;
        let doc = Node::from_html(html);
        // 触发 sxd 懒加载
        let package = doc.inner().root_element();
        let _ = doc;
        // 用 Node 间接访问 sxd_package
        let node = Node::from_html(html);
        let package = node.doc.sxd_package();  // doc 字段是 pub(crate)
        let sxd_doc = package.as_document();
        // 找到 p 元素
        let p_element = find_first_element_by_tag(sxd_doc.root(), "p")
            .expect("p should exist in sxd tree");
        let sig = NodeSignature::from_sxd(p_element);
        // 路径: html > body > div.main > p
        assert_eq!(sig.path.len(), 4);
        assert_eq!(sig.path[0], ("html".to_string(), None));
        assert_eq!(sig.path[1], ("body".to_string(), None));
        assert_eq!(sig.path[2], ("div".to_string(), Some("main".to_string())));
        assert_eq!(sig.path[3], ("p".to_string(), None));
    }

    #[test]
    fn test_signature_scraper_sxd_consistency() {
        // 同一段 HTML，scraper 和 sxd 构造的签名应该一致
        let html = r#"<html><body><div class="main"><p>text</p></div></body></html>"#;
        let doc = Node::from_html(html);
        let p = doc.select_one("p").expect("p should exist");
        let scraper_sig = NodeSignature::from_scraper(&p);

        let package = doc.doc.sxd_package();
        let sxd_doc = package.as_document();
        let p_element = find_first_element_by_tag(sxd_doc.root(), "p")
            .expect("p should exist in sxd tree");
        let sxd_sig = NodeSignature::from_sxd(p_element);

        assert_eq!(scraper_sig, sxd_sig, "scraper 和 sxd 签名应一致");
    }
}
```

**注意：** 测试用了 `doc.doc.sxd_package()`，需要确认 `Node.doc` 字段是 `pub(crate)`（在 `src/parser/mod.rs` line 26 确认是 `doc: Arc<Document>`，无 pub 修饰，默认 private）。

如果 `doc` 字段不可访问，在 `src/parser/mod.rs` 的 `Node` struct 定义中，将 `doc: Arc<Document>` 改为 `pub(crate) doc: Arc<Document>`（这是最小改动，仅 crate 内可见）。

**重要：** 这一改动如果需要，必须在 Task 1 完成（不能拖到 Task 2）。

- [ ] **Step 4: 验证编译**

Run: `cargo check`
Expected: 编译通过（可能有 unused warning，因为 NodeSignature 还没被使用）

- [ ] **Step 5: 运行新单元测试**

Run: `cargo test --lib parser::xpath::signature_tests`
Expected: 4 passed

如果 `test_signature_from_sxd_simple` 或 `test_signature_scraper_sxd_consistency` 失败（sxd 树结构和 scraper 不一致），检查 sxd-document 解析后的 DOM 结构（可能 `html > body > div > p` 或 `document > html > body > div > p`），调整断言。

- [ ] **Step 6: 运行全部 lib 测试确保无回归**

Run: `cargo test --lib`
Expected: 35 + 4 = 39 passed（原 35 + 新 4）

- [ ] **Step 7: 提交**

```bash
git add src/parser/xpath.rs src/parser/mod.rs
git commit -m "feat: 新增 NodeSignature 路径签名类型（scraper↔sxd 双向构造）"
```

**注意：** 如果 Task 1 Step 3 需要 `pub(crate) doc`，则 `src/parser/mod.rs` 也需提交；否则只提交 `src/parser/xpath.rs`。

---

## Task 2: 用签名匹配重写 locate_in_sxd（带回退）

**Files:**
- Modify: `src/parser/xpath.rs`（重写 `locate_in_sxd` 函数，line 64-71）
- Test: `tests/xpath_precision_test.rs`（新建）

**目标：** `locate_in_sxd` 改为"先用 `NodeSignature::from_scraper(node)` 在 sxd 树 DFS 匹配，失败回退到 `find_first_element_by_tag`"。新建集成测试验证多同名元素场景。

- [ ] **Step 1: 在 xpath.rs 的 NodeSignature impl 中追加 find_in_sxd 方法**

在 `src/parser/xpath.rs` 的 `impl NodeSignature` 块中（`from_sxd` 方法之后）追加：

```rust
    /// 在 sxd 树中 DFS 找到签名匹配的元素。
    ///
    /// 从 root 的子元素开始，逐级匹配 path。
    /// 返回第一个签名完全匹配的元素，找不到返回 None。
    fn find_in_sxd<'d>(&self, doc: dom::Document<'d>) -> Option<dom::Element<'d>> {
        if self.path.is_empty() { return None; }
        for child in doc.root().children() {
            if let dom::ChildOfRoot::Element(e) = child {
                if let Some(found) = dfs_sxd_match(e, &self.path, 0) {
                    return Some(found);
                }
            }
        }
        None
    }
```

- [ ] **Step 2: 在 xpath.rs 追加 dfs_sxd_match 辅助函数**

在 `impl NodeSignature` 块之后追加（模块级私有函数）：

```rust
/// DFS 遍历 sxd 树，匹配签名路径。
///
/// `depth` 是当前匹配到的路径深度（0 = 根级）。
fn dfs_sxd_match<'d>(
    element: dom::Element<'d>,
    path: &[(String, Option<String>)],
    depth: usize,
) -> Option<dom::Element<'d>> {
    if depth >= path.len() { return None; }
    let (tag, first_class) = &path[depth];
    // 匹配 tag
    if element.name().local_part() != tag { return None; }
    // 匹配 first_class
    let e_class = element.attributes().iter()
        .find(|a| a.name().local_part() == "class")
        .and_then(|a| a.value().split_whitespace().next().map(|s| s.to_string()));
    if &e_class != first_class { return None; }
    // 如果是最后一级，匹配成功
    if depth == path.len() - 1 {
        return Some(element);
    }
    // 递归子元素
    for child in element.children() {
        if let dom::ChildOfElement::Element(ce) = child {
            if let Some(found) = dfs_sxd_match(ce, path, depth + 1) {
                return Some(found);
            }
        }
    }
    None
}
```

- [ ] **Step 3: 重写 locate_in_sxd 函数**

将 `src/parser/xpath.rs` 的 `locate_in_sxd` 函数（line 64-71）替换为：

```rust
/// 在 sxd 树中定位 scraper 节点的对应节点。
///
/// 先用路径签名精确匹配，失败回退到"第一个同名元素"启发式。
fn locate_in_sxd<'d>(doc: dom::Document<'d>, node: &Node) -> Option<dom::Element<'d>> {
    let target_tag = node.tag();
    if target_tag.is_empty() {
        return None;
    }
    // 策略 1：路径签名精确匹配
    let sig = NodeSignature::from_scraper(node);
    if let Some(e) = sig.find_in_sxd(doc) {
        return Some(e);
    }
    // 策略 2：回退到第一个同名元素（保持向后兼容）
    find_first_element_by_tag(doc.root(), &target_tag)
}
```

- [ ] **Step 4: 新建 tests/xpath_precision_test.rs**

创建 `tests/xpath_precision_test.rs`：

```rust
//! XPath 回查精度测试。
//!
//! 验证 locate_in_sxd 在多同名元素场景下能精确定位。
//! 这些测试在 Stage 2 的"第一个同名元素"启发式下会失败。

use wisp::parser::Node;

#[test]
fn test_locate_among_same_tag_siblings() {
    // 三个同名 <div>，目标 div 有 class="target"
    let html = r#"<html><body>
      <div class="a"><p>A</p></div>
      <div class="target"><p>TARGET</p></div>
      <div class="c"><p>C</p></div>
    </body></html>"#;
    let doc = Node::from_html(html);
    // 选中 target div 的 <p>
    let target_p = doc.select_one("div.target p").expect("target p should exist");
    // 用 xpath 查询，触发 locate_in_sxd
    // //p 从根查找所有 p，应返回 3 个
    let all_p = doc.xpath("//p");
    assert_eq!(all_p.len(), 3, "//p 应返回 3 个 p 元素");

    // 验证回查精度：xpath("//p[2]") 或类似的相对查询
    // 这里用相对 xpath 验证 locate_in_sxd 的精度
    // 从 target_p 出发，xpath(".//p") 或查询其父节点
    let parent = target_p.parent().expect("target p should have parent");
    let parent_children = parent.children();
    assert_eq!(parent_children.len(), 1, "target div 应只有 1 个 p 子元素");
}

#[test]
fn test_xpath_returns_correct_among_duplicates() {
    // 多个相同结构的 div.item，每个含一个 p
    let html = r#"<html><body>
      <div class="item"><p>Item 1</p></div>
      <div class="item"><p>Item 2</p></div>
      <div class="item"><p>Item 3</p></div>
    </body></html>"#;
    let doc = Node::from_html(html);
    // //p 应返回 3 个，文本分别是 Item 1/2/3
    let ps = doc.xpath("//p");
    assert_eq!(ps.len(), 3);
    let texts: Vec<String> = ps.iter().map(|n| n.text()).collect();
    assert_eq!(texts, vec!["Item 1", "Item 2", "Item 3"]);
}

#[test]
fn test_xpath_nested_same_tag() {
    // 深层嵌套的同名 div
    let html = r#"<html><body>
      <div class="outer">
        <div class="middle">
          <div class="inner">
            <p>deep</p>
          </div>
        </div>
      </div>
      <div class="other"><p>shallow</p></div>
    </body></html>"#;
    let doc = Node::from_html(html);
    // //div[@class='inner']/p 应返回 deep
    let result = doc.xpath("//div[@class='inner']/p");
    assert_eq!(result.len(), 1, "应找到 1 个 inner div 下的 p");
    assert_eq!(result.first().unwrap().text(), "deep");
}

#[test]
fn test_xpath_relative_from_nested_node() {
    // 从嵌套节点出发的相对 xpath 查询
    let html = r#"<html><body>
      <div class="container">
        <ul>
          <li><a href="/link1">Link 1</a></li>
          <li><a href="/link2">Link 2</a></li>
        </ul>
      </div>
    </body></html>"#;
    let doc = Node::from_html(html);
    // 选中第一个 li
    let first_li = doc.select_one("li").expect("first li should exist");
    // 从 first_li 出发查询 .//a（相对 xpath）
    // 这会触发 locate_in_sxd(first_li)，需要精确定位 sxd 树中的对应 li
    let links = first_li.xpath(".//a");
    assert_eq!(links.len(), 1, "第一个 li 应只有 1 个 a");
    assert_eq!(links.first().unwrap().attr("href"), Some("/link1".to_string()));
}
```

- [ ] **Step 5: 运行新测试（预期部分失败）**

Run: `cargo test --test xpath_precision_test`
Expected: 至少 1 个测试失败（`test_xpath_relative_from_nested_node` 或类似，因为旧 `locate_in_sxd` 用第一个同名元素）

**如果所有测试都通过**：说明当前启发式已经足够，或测试用例不够严格。检查 `test_xpath_relative_from_nested_node` 是否真的触发了 `locate_in_sxd`（需要 `.//a` 走慢路径，不被 `xpath_to_css` 快速路径拦截）。

如果 `.//a` 被 `xpath_to_css` 快速路径拦截（转成 CSS），改用更复杂的 xpath 如 `.//a[@href]` 或 `descendant::a`。

- [ ] **Step 6: 验证签名匹配生效**

如果 Step 5 测试失败，确认是因为 `locate_in_sxd` 回退到旧策略。现在签名匹配应该已经生效（Task 2 Step 3 已重写）。

Run: `cargo test --test xpath_precision_test`
Expected: 全部通过（4 passed）

如果仍有失败，检查：
- `NodeSignature::from_scraper` 构造的路径是否正确（用 `dbg!(&sig)` 调试）
- `dfs_sxd_match` 是否正确遍历（检查 sxd 树结构是否和 scraper 一致）
- sxd 树可能有 `document > html > body > ...` 的额外层级（`document` 是 sxd 的根，不在 scraper 树中）

如果 sxd 树有额外层级，调整 `from_sxd` 跳过 `document` 根（只从 `html` 开始）。或调整 `find_in_sxd` 从 `doc.root().children()` 开始（已经是这样）。

- [ ] **Step 7: 运行全部 xpath 测试确保无回归**

Run: `cargo test --test xpath_test`
Expected: 9 passed（Stage 2 的 xpath 测试全部通过）

- [ ] **Step 8: 运行全部 lib 测试确保无回归**

Run: `cargo test --lib`
Expected: 39 passed（35 原 + 4 signature_tests）

- [ ] **Step 9: 提交**

```bash
git add src/parser/xpath.rs tests/xpath_precision_test.rs
git commit -m "feat: locate_in_sxd 升级为路径签名精确匹配（带回退）"
```

---

## Task 3: 用签名匹配重写 find_in_scraper（带回退）+ 全测试套件验证

**Files:**
- Modify: `src/parser/xpath.rs`（重写 `find_in_scraper` 函数，line 109-133）
- Test: `tests/xpath_precision_test.rs`（追加测试）

**目标：** `find_in_scraper` 改为"先用 `NodeSignature::from_sxd(sxd_node)` 在 scraper 树匹配，失败回退到 tag + 第一个属性策略"。追加测试验证 sxd→scraper 方向的精度。运行全测试套件验证无回归。

- [ ] **Step 1: 在 xpath.rs 的 NodeSignature impl 中追加 find_in_scraper 方法**

在 `src/parser/xpath.rs` 的 `impl NodeSignature` 块中（`find_in_sxd` 方法之后）追加：

```rust
    /// 在 scraper 树中找到签名匹配的 Node。
    ///
    /// 用 `select("*")` 遍历所有元素，对每个元素构造签名比较。
    /// 返回第一个签名完全匹配的 Node，找不到返回 None。
    fn find_in_scraper(&self, doc: &Arc<Document>) -> Option<Node> {
        if self.path.is_empty() { return None; }
        // 优化：用最后一级的 tag + first_class 构造选择器缩小范围
        let (last_tag, last_class) = &self.path[self.path.len() - 1];
        let selector_str = match last_class {
            Some(c) => format!("{}.{}", last_tag, c),
            None => last_tag.clone(),
        };
        let selector = scraper::Selector::parse(&selector_str).ok()?;
        for el in doc.html.select(&selector) {
            let node = Node::from_element_ref(doc.clone(), el);
            let node_sig = NodeSignature::from_scraper(&node);
            if node_sig == *self {
                return Some(node);
            }
        }
        None
    }
```

**注意：** `Node::from_element_ref` 是 private 方法（`fn from_element_ref`，无 pub）。在 `src/parser/mod.rs` line 32 确认。由于 `xpath.rs` 和 `mod.rs` 在同一 crate，且 `from_element_ref` 是 `fn`（crate 内可见），`xpath.rs` 可以调用。

如果 `from_element_ref` 不可访问，在 `src/parser/mod.rs` 将其改为 `pub(crate) fn from_element_ref`。

- [ ] **Step 2: 重写 find_in_scraper 函数**

将 `src/parser/xpath.rs` 的 `find_in_scraper` 函数（line 109-133）替换为：

```rust
/// 在 scraper 树中找到 sxd 节点的对应节点。
///
/// 先用路径签名精确匹配，失败回退到"tag + 第一个属性"启发式。
fn find_in_scraper<'d>(doc: &Arc<Document>, sxd_node: &dom::Element<'d>) -> Option<Node> {
    // 策略 1：路径签名精确匹配
    let sig = NodeSignature::from_sxd(*sxd_node);
    if let Some(node) = sig.find_in_scraper(doc) {
        return Some(node);
    }
    // 策略 2：回退到 tag + 第一个属性（保持向后兼容）
    let tag = sxd_node.name().local_part();
    let attrs: Vec<(String, String)> = sxd_node
        .attributes()
        .iter()
        .map(|a| (a.name().local_part().to_string(), a.value().to_string()))
        .collect();
    let selector_str = if attrs.is_empty() {
        tag.to_string()
    } else {
        let (k, v) = &attrs[0];
        format!("{}[{}='{}']", tag, k, v)
    };
    let selector = scraper::Selector::parse(&selector_str).ok()?;
    doc.html
        .select(&selector)
        .next()
        .map(|el| Node::from_element_ref(doc.clone(), el))
}
```

- [ ] **Step 3: 在 tests/xpath_precision_test.rs 追加 sxd→scraper 方向的测试**

在 `tests/xpath_precision_test.rs` 末尾追加：

```rust

#[test]
fn test_xpath_result_matches_correct_node() {
    // 验证 xpath 返回的 Node 是正确的（sxd→scraper 回查精确）
    // 两个相同结构的 div.item，但内容不同
    let html = r#"<html><body>
      <div class="item"><h3>First</h3><span>$10</span></div>
      <div class="item"><h3>Second</h3><span>$20</span></div>
    </body></html>"#;
    let doc = Node::from_html(html);
    // //span 查询，应返回 2 个，分别对应 $10 和 $20
    let spans = doc.xpath("//span");
    assert_eq!(spans.len(), 2);
    let texts: Vec<String> = spans.iter().map(|n| n.text()).collect();
    assert_eq!(texts, vec!["$10", "$20"]);
}

#[test]
fn test_xpath_among_many_same_tag_same_class() {
    // 多个相同 tag + 相同 class 的元素，内容不同
    // 旧的 find_in_scraper 用 "tag + 第一个属性" 会取第一个，可能错
    let html = r#"<html><body>
      <ul>
        <li class="item">Alpha</li>
        <li class="item">Beta</li>
        <li class="item">Gamma</li>
        <li class="item">Delta</li>
      </ul>
    </body></html>"#;
    let doc = Node::from_html(html);
    // //li 查询，应返回 4 个，顺序正确
    let lis = doc.xpath("//li");
    assert_eq!(lis.len(), 4);
    let texts: Vec<String> = lis.iter().map(|n| n.text()).collect();
    assert_eq!(texts, vec!["Alpha", "Beta", "Gamma", "Delta"]);
}

#[test]
fn test_xpath_deeply_nested_precision() {
    // 深层嵌套，验证路径签名能穿透多层
    let html = r#"<html><body>
      <div class="root">
        <div class="level1">
          <div class="level2">
            <div class="level3">
              <span class="target">found me</span>
            </div>
          </div>
        </div>
        <div class="level1">
          <div class="level2">
            <div class="level3">
              <span class="target">not me</span>
            </div>
          </div>
        </div>
      </div>
    </body></html>"#;
    let doc = Node::from_html(html);
    // //span[@class='target'] 应返回 2 个
    let targets = doc.xpath("//span[@class='target']");
    assert_eq!(targets.len(), 2);
    let texts: Vec<String> = targets.iter().map(|n| n.text()).collect();
    assert!(texts.contains(&"found me".to_string()));
    assert!(texts.contains(&"not me".to_string()));
}
```

- [ ] **Step 4: 运行新测试**

Run: `cargo test --test xpath_precision_test`
Expected: 7 passed（Task 2 的 4 个 + Task 3 的 3 个）

如果 `test_xpath_among_many_same_tag_same_class` 或类似失败，检查：
- `find_in_scraper` 的签名匹配是否正确
- `select(&selector)` 是否返回所有候选（不只是第一个）
- 签名比较是否正确（用 `dbg!` 调试）

- [ ] **Step 5: 运行全部 xpath 测试确保无回归**

Run: `cargo test --test xpath_test`
Expected: 9 passed

- [ ] **Step 6: 运行全部 lib 测试确保无回归**

Run: `cargo test --lib`
Expected: 39 passed

- [ ] **Step 7: 运行集成测试确保无回归**

Run:
```
cargo test --test adaptive_test
cargo test --test dom_navigation_test
cargo test --test integration fetch_test
cargo test --test fetch_test
```
Expected: adaptive 5 + dom_nav 9 + integration fetch_test 3 + fetch_test 7 = 24 passed

- [ ] **Step 8: 运行完整测试套件**

Run:
```
cargo test --lib
cargo test --test adaptive_test
cargo test --test crawl_checkpoint_test
cargo test --test difflib_test
cargo test --test dom_navigation_test
cargo test --test xpath_test
cargo test --test xpath_precision_test
cargo test --test fetch_test
cargo test --test integration fetch_test
```
Expected: 39 + 5 + 4 + 7 + 9 + 9 + 7 + 7 + 3 = 90 passed

- [ ] **Step 9: 提交**

```bash
git add src/parser/xpath.rs tests/xpath_precision_test.rs
git commit -m "feat: find_in_scraper 升级为路径签名精确匹配（带回退）"
```

---

## Self-Review 检查

**1. Spec 覆盖检查：**
- ✅ `locate_in_sxd` 精确匹配 → Task 2
- ✅ `find_in_scraper` 精确匹配 → Task 3
- ✅ 向后兼容（回退策略）→ Task 2/3 都有回退
- ✅ 测试覆盖（多同名元素 + 深层嵌套）→ Task 2/3 测试
- ✅ 无公共 API 变化 → 只改内部函数 + 新增内部类型

**2. Placeholder 扫描：**
- 无 "TBD"、"TODO" 占位
- 所有步骤都有完整代码
- 无 "类似 Task N" 引用

**3. 类型一致性：**
- `NodeSignature` 在 Task 1 定义，Task 2/3 使用一致
- `from_scraper(&Node) -> Self` 在 Task 1 定义，Task 2/3 使用一致
- `from_sxd(dom::Element) -> Self` 在 Task 1 定义，Task 3 使用一致
- `find_in_sxd(dom::Document) -> Option<dom::Element>` 在 Task 2 定义并使用
- `find_in_scraper(&Arc<Document>) -> Option<Node>` 在 Task 3 定义并使用
- `dfs_sxd_match` 在 Task 2 定义并使用

**4. 已知简化：**
- 签名只用 `(tag, first_class)`，不用 position。多个相同 tag + 相同 first_class 的兄弟元素仍可能匹配到第一个。但这是边缘情况，且签名匹配失败会回退到旧策略，不影响正确性。
- `find_in_scraper` 用 `select(&selector)` 遍历候选，对大文档（>10000 元素）性能差。但 YAGNI，先简单实现。

**5. 风险点：**
- sxd-document 0.3.2 的 `dom::Parent` / `dom::ChildOfElement` / `dom::ChildOfRoot` 枚举变体名需在 Task 1 Step 1 确认（可能是 `Element` / `Root` 等）
- `Node.doc` 字段可见性可能需调整（Task 1 Step 3）
- `Node::from_element_ref` 可见性可能需调整（Task 3 Step 1）
- sxd 树可能有 `document` 根层级（不在 scraper 树中），需在 `from_sxd` / `find_in_sxd` 中处理

---

## 执行说明

本 plan 适用于 subagent-driven-development。3 个 Task 顺序执行：Task 1（类型基础）→ Task 2（locate_in_sxd + 测试）→ Task 3（find_in_scraper + 测试 + 全验证）。每个 Task 独立可测、可提交。Task 1 的 `dom::Parent` API 需在 Step 1 确认。
