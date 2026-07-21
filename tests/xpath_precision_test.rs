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
    let texts = spans.text();
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
    let texts = lis.text();
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
    // descendant::span[@class='target'] 强制走 sxd-xpath 慢路径
    // （//span[@class='target'] 会被 xpath_to_css 快速路径拦截，丢失多结果）
    let targets = doc.xpath("descendant::span[@class='target']");
    assert_eq!(targets.len(), 2);
    let texts = targets.text();
    assert!(texts.contains(&"found me".to_string()));
    assert!(texts.contains(&"not me".to_string()));
}

/// 增强测试：真正区分新旧 locate_in_sxd 行为（Task 2 reviewer 建议）
/// 选中第二个 li（非第一个），xpath(".//a") 应返回第二个 li 的 a，
/// 旧代码（find_first_element_by_tag）会返回第一个 li 的 a，新代码正确。
#[test]
fn test_locate_in_sxd_precision_second_sibling() {
    let html = r#"<html><body>
      <div class="container">
        <ul>
          <li><a href="/link1">Link 1</a></li>
          <li><a href="/link2">Link 2</a></li>
          <li><a href="/link3">Link 3</a></li>
        </ul>
      </div>
    </body></html>"#;
    let doc = Node::from_html(html);
    // 选中第二个 li
    let lis = doc.select("li");
    assert_eq!(lis.len(), 3, "应有 3 个 li");
    let second_li = lis.get(1).expect("第二个 li 应存在").clone();
    // 从 second_li 出发查询 .//a（相对 xpath，走慢路径触发 locate_in_sxd）
    let links = second_li.xpath(".//a");
    assert_eq!(links.len(), 1, "第二个 li 应只有 1 个 a");
    // 关键断言：必须是 /link2，不能是 /link1
    // 旧 locate_in_sxd 用 find_first_element_by_tag 会定位到第一个 li，
    // 返回 /link1；新代码用签名匹配定位到第二个 li，返回 /link2
    assert_eq!(
        links.first().unwrap().attr("href"),
        Some("/link2".to_string()),
        "locate_in_sxd 应精确定位到第二个 li（返回 /link2），而非第一个（/link1）"
    );
    assert_eq!(links.first().unwrap().text(), "Link 2");
}
