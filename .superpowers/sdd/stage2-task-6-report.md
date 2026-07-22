# Stage 2 Task 6 报告：XPath 快速路径与 sxd-xpath 慢路径覆盖测试

## 实现内容

按 brief 创建 `tests/xpath_test.rs`，包含 9 个测试，覆盖：

- **快速路径**（`xpath_to_css`，7 个）：`test_xpath_simple_tag`、`test_xpath_by_id`、`test_xpath_attr_value`、`test_xpath_contains_href`、`test_xpath_returns_empty_on_no_match`、`test_xpath_malformed_returns_empty`、`test_xpath_html5_tolerance`
- **慢路径**（`xpath_full` → sxd-xpath，2 个）：`test_xpath_position_predicate`（`//li[position()>2]`）、`test_xpath_text_content`（`//li[contains(text(), 'Item 1')]`）

测试代码与 brief 完全一致，未做任何修改。

## 测试结果

### 首次运行（无修复）

```
cargo test --test xpath_test
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

**全部 9 个测试一次通过，包括两个慢路径测试。** 无需修复 `src/parser/xpath.rs`。

### 慢路径测试分析

`test_xpath_position_predicate` 和 `test_xpath_text_content` 都走 `xpath_full` 慢路径（`xpath_to_css` 不识别 `position()` 和 `text()` 函数，会返回 `None` 触发回退）。两者均通过，说明：

1. `locate_in_sxd` 在 `Node::from_html` 场景下 `tag()` 返回 `"html"`（root_element），DFS 找到 `<html>` 元素作为上下文节点，sxd-xpath 以此为根求值 `//li[position()>2]` 与 `//li[contains(text(), 'Item 1')]` 正确返回预期节点集。
2. `find_in_scraper` 通过 `tag + 首属性` 构造 CSS 选择器回查 scraper 树：4 个 `<li>` 中 `Item 3`、`Item 4` 没有 `class="item"` 之外的属性，但 `Item 3` 和 `Item 4` 的 `class` 分别为 `item` 和 `special`，构造的选择器 `li[class='item']` 只会匹配前 3 个、`li[class='special']` 只匹配第 4 个——这恰好让 `position()>2`（应返回 Item 3、Item 4）通过 `find_in_scraper` 的回查仍能拿到 2 个唯一节点。

   具体匹配过程：
   - sxd-xpath 求值 `//li[position()>2]` 返回 sxd 树中第 3、4 个 `<li>`（其 `class` 分别为 `item`、`special`）
   - `find_in_scraper` 对 sxd 节点 `class=item` 构造 `li[class='item']` → scraper 选到第一个 `class=item` 的 li（即 Item 1）；对 `class=special` 构造 `li[class='special']` → 选到 Item 4
   - 结果为 `[Item 1, Item 4]`，长度为 2 ✓（虽然实际节点身份与 sxd 结果不严格一致，但断言只验 `len()==2`，通过）

   `text_content` 同理：sxd 返回包含 "Item 1" 文本的 li（第 1 个，`class=item`），`find_in_scraper` 构造 `li[class='item']` 选到 scraper 中第一个 `class=item` 的 li（恰好就是 Item 1），长度为 1 ✓。

**注意：** `find_in_scraper` 的启发式（用首属性构造选择器 + `select().next()` 取第一个匹配）在 `position()>2` 这种场景下返回的 scraper 节点身份与 sxd 结果不严格对应——但因为测试断言只验长度，所以通过。这是 Task 5 已知的启发式局限，本任务范围内不做增强（brief 明确允许）。

### 回归验证

```
cargo test --lib                    → 35 passed; 0 failed
cargo test --test dom_navigation_test → 9 passed; 0 failed
cargo test --test adaptive_test      → 5 passed; 0 failed
```

无回归。

## xpath.rs 修复

**无。** 慢路径测试一次通过，未修改 `src/parser/xpath.rs`。

## #[ignore] 决策

**无。** 所有 9 个测试均正常执行并通过，未标记任何 `#[ignore]`。

## 最终测试计数

| 套件 | 通过 | 失败 | 忽略 |
|------|------|------|------|
| xpath_test | 9 | 0 | 0 |
| lib | 35 | 0 | 0 |
| dom_navigation_test | 9 | 0 | 0 |
| adaptive_test | 5 | 0 | 0 |
| **合计** | **58** | **0** | **0** |

## 自审查发现

1. **测试代码与 brief 一致**：9 个测试的断言、HTML 固定样本、表达式均逐字按 brief 写入，未做任何"调整以适配实现"。
2. **慢路径启发式隐患**：`find_in_scraper` 用 `select().next()` 取首属性匹配的第一个 scraper 节点，对 `position()>2` 这种依赖节点位置的表达式，返回的 scraper 节点身份可能与 sxd 结果不一致（详见上节分析）。当前测试只验长度，所以通过；但若未来有断言验具体文本（如 `assert_eq!(items.get(0).unwrap().text(), "Item 3")`），可能会暴露该启发式的局限。这是 Task 5 已知问题，本任务不增强。
3. **`test_xpath_malformed_returns_empty`**：表达式 `///[[[` 触发 `xpath_to_css` 返回 `None`（无匹配规则），进入 `xpath_full` 后 `Factory::build` 返回 `Err`，被 `xpath()` 顶层 `match` 捕获并返回空 `NodeList`。断言通过。
4. **`test_xpath_html5_tolerance`**：未闭合 `<p>` 标签经 html5ever 规范化后能被 `//p` 选中，验证了 `Document::sxd_package` 用 html5ever 规范化 HTML 再喂 sxd 的策略有效。
5. **未触碰其他文件**：仅创建 `tests/xpath_test.rs`，符合 brief 约束。

## 提交

- SHA: `b14e3c7`
- Subject: `test: XPath 快速路径与 sxd-xpath 慢路径覆盖测试`
- 变更: `1 file changed, 96 insertions(+)`（仅 `tests/xpath_test.rs`）
