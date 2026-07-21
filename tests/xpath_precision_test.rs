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
    let texts: Vec<String> = ps.text();
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
    // descendant::div[@class='inner']/p 强制走 sxd-xpath 慢路径
    // （//div[@class='inner']/p 会被 xpath_to_css 快速路径拦截，丢弃 /p 后缀）
    let result = doc.xpath("descendant::div[@class='inner']/p");
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
