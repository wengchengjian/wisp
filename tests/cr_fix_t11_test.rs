//! Task 11 回归测试：xpath 签名失败不再回退启发式。
//!
//! 验证 locate_in_sxd / find_in_scraper 移除启发式回退后，
//! xpath 查询在多同名元素场景下仍精确返回正确节点。
//! 旧的 find_in_scraper 启发式（tag + 第一个属性）在无属性的 <p> 上会回退到
//! `p` 选择器，返回文档中第一个 <p>，导致节点错位。

use wisp::parser::Node;

#[test]
fn test_xpath_signature_failure_returns_none_not_heuristic() {
    // 两个同名 div（仅 id 不同，无 class），各含一个无属性的 <p>
    // find_in_scraper 旧启发式会构造 `p` 选择器取第一个 <p>，返回 "first"
    // 签名精确匹配应定位到第二个 div 下的 <p>，返回 "second"
    let html = r#"<html><body>
        <div id="a"><p>first</p></div>
        <div id="b"><p>second</p></div>
    </body></html>"#;
    let doc = Node::from_html(html);
    // //div[@id='b']/p 走 sxd-xpath 慢路径（带 /p 后缀，不会被快速路径拦截）
    let nodes = doc.xpath("//div[@id='b']/p");
    assert_eq!(nodes.len(), 1, "应只返回 1 个节点");
    let text = nodes.get(0).unwrap().text();
    assert_eq!(
        text.trim(),
        "second",
        "应匹配第二个 div 的 p，不是回退到第一个"
    );
}

#[test]
fn test_find_in_scraper_no_attribute_heuristic_fallback() {
    // 多个无属性的同名元素，sxd 结果回查应精确匹配 sibling 索引
    // 旧 find_in_scraper 对无属性元素回退到裸 tag 选择器，总是返回第一个
    let html = r#"<html><body>
        <ul>
            <li><span>A</span></li>
            <li><span>B</span></li>
            <li><span>C</span></li>
        </ul>
    </body></html>"#;
    let doc = Node::from_html(html);
    // descendant::span 强制走 sxd-xpath 慢路径
    let spans = doc.xpath("descendant::span");
    assert_eq!(spans.len(), 3, "应返回 3 个 span");
    let texts: Vec<String> = spans.text();
    // 验证返回 3 个不同的节点（旧启发式会返回 3 个相同的"第一个"）
    let unique: std::collections::HashSet<&String> = texts.iter().collect();
    assert_eq!(unique.len(), 3, "3 个 span 应各不相同，不应因启发式回退返回重复节点");
    assert!(unique.contains(&"A".to_string()), "应包含 A");
    assert!(unique.contains(&"B".to_string()), "应包含 B");
    assert!(unique.contains(&"C".to_string()), "应包含 C");
}
