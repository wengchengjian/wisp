# Stage 2 Task 4 报告：真实实现 parent/children/sibling/ancestors/matches DOM 导航

## 实现内容

在 `src/parser/mod.rs` 的 `Node` impl 中替换 Task 3 的临时实现，并新增 `ancestors()`：

| 方法 | 实现方式 |
|------|---------|
| `parent()` | `element.parent().and_then(ElementRef::wrap).map(...)` — 使用备选方案，因 scraper 0.23 `Element` 无 `is_element()` |
| `next_sibling()` | 循环 `element.next_sibling()` + `ElementRef::wrap` 过滤非元素节点（文本/注释） |
| `prev_sibling()` | 循环 `element.prev_sibling()` + `ElementRef::wrap` 过滤非元素节点 |
| `ancestors()` (新增) | `std::iter::successors(self.parent(), \|node\| node.parent())` 惰性迭代 |
| `matches()` | `CssSelector::parse(css)` → `selector.matches(&element_ref)` |

`children()` 和 `tag()` 在 Task 3 已实现，本次未改动。

## TDD 证据

### RED 阶段（实现前）

命令：`cargo test --test dom_navigation_test`

结果：**编译失败**（exit code 101），错误：
```
error[E0599]: no method named `ancestors` found for struct `Node` in the current scope
  --> tests\dom_navigation_test.rs:67:34
```

未实现的 API：
- `ancestors()` — 方法不存在（编译失败）
- `parent()` / `next_sibling()` / `prev_sibling()` — 返回 `None`
- `matches()` — 返回 `false`

预期失败的测试（若编译通过）：
- `test_parent_navigation`
- `test_next_sibling`
- `test_prev_sibling`
- `test_next_sibling_none_at_end`
- `test_ancestors_iterator`
- `test_matches_simple_selector`
- `test_matches_compound_selector`

预期通过的测试（Task 3 已实现 children/tag）：
- `test_children_navigation`
- `test_tag_name`

### GREEN 阶段（实现后）

命令：`cargo test --test dom_navigation_test`

结果：**9 passed; 0 failed**
```
test test_parent_navigation ... ok
test test_prev_sibling ... ok
test test_next_sibling_none_at_end ... ok
test test_children_navigation ... ok
test test_next_sibling ... ok
test test_ancestors_iterator ... ok
test test_tag_name ... ok
test test_matches_simple_selector ... ok
test test_matches_compound_selector ... ok
```

## 测试结果汇总

| 套件 | 命令 | 结果 |
|------|------|------|
| dom_navigation | `cargo test --test dom_navigation_test` | 9 passed |
| lib | `cargo test --lib` | 35 passed |
| adaptive | `cargo test --test adaptive_test` | 5 passed |
| difflib | `cargo test --test difflib_test` | 7 passed |

**总计：56 测试全部通过，无回归。**

## scraper 0.23 API 调整

| Brief 建议 | 实际 API | 调整 |
|-----------|---------|------|
| `element.value().matches(&selector)` | `selector.matches(&element_ref)` | **关键调整**：`matches` 方法在 `Selector` 上，不在 `Element` 上。签名 `Selector::matches(&self, &ElementRef) -> bool`（scraper 0.23.1 `src/selector.rs:41`） |
| `element.parent().and_then(ElementRef::wrap).filter(\|p\| p.value().is_element())` | `element.parent().and_then(ElementRef::wrap)` | 用备选方案：`ElementRef::wrap` 本身已过滤非元素节点，无需 `is_element()`（该方法不存在） |
| `next_sibling` / `prev_sibling` 中 `el.value().is_element()` | `ElementRef::wrap(s).is_some()` | 用备选方案：直接靠 `ElementRef::wrap` 过滤 |

## Self-review 发现

1. **parent() 对文档根节点**：`<html>` 的 parent 是文档根（非元素），`ElementRef::wrap` 返回 `None`，故 `parent()` 返回 `None`。`ancestors()` 迭代到 `<html>` 后自然终止，符合预期。
2. **next_sibling 跨文本节点**：HTML 中 `<p>` 之间有空白文本节点，循环 `next_sibling()` 跳过它们直到找到下一个元素。`test_next_sibling` 验证了此行为。
3. **matches 无效选择器**：`CssSelector::parse` 失败时返回 `false` 而非 panic，符合 Rust idiom。
4. **ancestors 惰性迭代**：使用 `std::iter::successors` 返回 `impl Iterator`，无内存分配，可提前 break。
5. **API 签名保持不变**：所有公共方法签名与 Task 3 一致，未破坏调用方。

## 提交

- SHA: `89471d86ed0d8cc1b0a8353820549f2eda90f2ee`
- Subject: `feat: 真实实现 parent/children/sibling/ancestors/matches DOM 导航`
- Files: `src/parser/mod.rs` (+62/-7), `tests/dom_navigation_test.rs` (+135 新文件)
