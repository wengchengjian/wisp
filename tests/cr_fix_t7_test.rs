//! Task 7 回归测试：adaptive helpers 用 Node 导航替代重复解析。
//!
//! 验证 `similarity()` 不再对每个候选节点调用 4 次 `Html::parse_document`，
//! 通过性能特征间接验证（100 次 similarity 应 < 500ms）。
//!
//! 旧实现：每次 `similarity` 调用 4 个 helper（`node_tag_name`/`ancestor_path_of`/
//! `sibling_tags_of`/`parent_attrs_of`），每个 helper 都 `outer_html()` +
//! `Html::parse_document()` 重新解析 HTML，即每候选节点解析 4 次。
//! 新实现：直接用 `Node::tag()`/`ancestors()`/`parent()`/`children()`/`attrs()`，
//! 零次 HTML 重解析。

use std::time::Instant;
use wisp::parser::{Node, ElementSnapshot, similarity};

#[test]
fn test_similarity_uses_node_navigation_not_reparse() {
    let html = r#"<html><body>
        <div class="products">
            <ul class="list">
                <li class="item"><span>Product A</span></li>
                <li class="item"><span>Product B</span></li>
                <li class="item"><span>Product C</span></li>
            </ul>
        </div>
    </body></html>"#;

    let doc = Node::from_html(html);
    // select_all 返回拥有所有权的 Vec<Node>，避免借用临时 NodeList
    let li = doc.select_all("li.item").into_iter().next()
        .expect("应找到 li.item");

    // 捕获快照（capture 也用 Node 导航 API，不重解析）
    let snapshot = ElementSnapshot::capture(&li);

    // 验证快照捕获的导航数据非空（确认 Node 导航 API 真实工作）
    assert!(!snapshot.tag.is_empty(), "tag 应非空");
    assert!(
        !snapshot.ancestor_path.is_empty(),
        "ancestor_path 应非空（Node::ancestors 工作）"
    );
    assert!(
        !snapshot.sibling_tags.is_empty(),
        "sibling_tags 应非空（Node::parent/children 工作）"
    );
    assert_eq!(
        snapshot.sibling_tags,
        vec!["li".to_string(), "li".to_string(), "li".to_string()],
        "sibling_tags 应为 3 个 li"
    );

    // 100 次 similarity 应 < 500ms。
    // 旧实现每次 similarity 调用 4 个 helper，每个 helper 都 outer_html + parse_document，
    // 100 次 = 400 次 HTML 解析，会远超 500ms。
    // 新实现零次 HTML 解析，应远低于阈值。
    let start = Instant::now();
    for _ in 0..100 {
        let score = similarity(&li, &snapshot);
        // 自相似应为 1.0（完全匹配）
        assert!(
            (score - 1.0).abs() < 1e-9,
            "自相似度应为 1.0，实际 {}",
            score
        );
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed.as_millis() < 500,
        "100 次 similarity 应 < 500ms，实际 {:?}",
        elapsed
    );
}

#[test]
fn test_similarity_helpers_match_capture_logic() {
    // 验证 similarity 内部 helpers 与 ElementSnapshot::capture 逻辑一致：
    // 对同一节点，helpers 计算的 ancestor_path/sibling_tags/parent_attrs 应与 snapshot 一致。
    let html = r##"<html><body>
        <div class="main">
            <ul class="nav">
                <li class="item active"><a href="#">Home</a></li>
                <li class="item"><a href="#">About</a></li>
            </ul>
        </div>
    </body></html>"##;

    let doc = Node::from_html(html);
    let li = doc.select_all("li.item.active").into_iter().next()
        .expect("应找到 li.item.active");
    let snapshot = ElementSnapshot::capture(&li);

    // similarity 内部用 helpers 计算 node 的维度，与 snapshot（也用 Node API）应一致。
    // 自相似度 = 1.0 证明 helpers 与 capture 数据完全吻合。
    let score = similarity(&li, &snapshot);
    assert!(
        (score - 1.0).abs() < 1e-9,
        "自相似度应为 1.0（helpers 与 capture 一致），实际 {}",
        score
    );

    // 验证 parent_attrs 捕获了父节点（ul.nav）的 class
    assert_eq!(
        snapshot.parent_attrs.get("class").map(|s| s.as_str()),
        Some("nav"),
        "parent_attrs 应包含父节点 ul 的 class=nav"
    );
}
