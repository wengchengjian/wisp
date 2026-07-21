//! 真实环境测试：robots.txt 解析。cargo test --test crawl_robots_real_test -- --ignored 运行。

use wisp::fetch::Client;
use wisp::crawl::robots::RobotsCache;

#[tokio::test]
#[ignore = "requires network access"]
async fn test_real_robots_txt_parses_crawl_delay() {
    // example.com 的 robots.txt 可能含或不含 Crawl-delay，不断言具体值
    // 仅验证解析过程不 panic 且返回 RobotsRules
    let client = Client::new().unwrap();
    let mut cache = RobotsCache::new();
    let rules = cache.rules_for(&client, "https://example.com/").await;
    println!("example.com rules: {:?}", rules);
    // rules_for 不应 panic，且字段类型符合预期
    let _delay = rules.crawl_delay;
    let _rate = rules.request_rate;
    let _disallowed = &rules.disallowed;
}

#[tokio::test]
#[ignore = "requires network access"]
async fn test_real_robots_txt_disallow_respected() {
    let client = Client::new().unwrap();
    let mut cache = RobotsCache::new();
    // httpbin.org 的 robots.txt 应允许 /status/200（不被 Disallow 阻止）
    let allowed = cache.is_allowed(&client, "https://httpbin.org/status/200").await;
    println!("httpbin.org/status/200 allowed: {}", allowed);
    // 不断言具体值（站点 robots 可能变化），仅验证不 panic
}

#[tokio::test]
#[ignore = "requires network access"]
async fn test_real_robots_txt_crawl_delay_returns_some_or_none() {
    // 验证 crawl_delay() 接口可用，返回 Option<f64>
    let client = Client::new().unwrap();
    let mut cache = RobotsCache::new();
    let delay = cache.crawl_delay(&client, "https://example.com/").await;
    println!("example.com crawl_delay: {:?}", delay);
    // 不断言具体值
}
