//! Engine 纯基础设施测试（Task 3）：
//! - 多次 run 不同 Spider 共享底层资源
//! - 多 Engine 实例控制状态隔离
//! - run 返回 (stats, items)
//! - 每次 run 重置 control 状态

use wisp::crawl::*;
use async_trait::async_trait;
use serde_json::Value;

/// 最小 Spider：parse 返回单个 item，不 follow。
struct CountSpider {
    name: String,
    url: String,
}

#[async_trait]
impl Spider for CountSpider {
    fn name(&self) -> &str {
        &self.name
    }
    fn start_urls(&self) -> Vec<String> {
        vec![self.url.clone()]
    }
    async fn parse(&self, _resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
        (vec![serde_json::json!({"name": self.name})], vec![])
    }
    fn obey_robots(&self) -> bool {
        false
    }
    fn max_retries(&self) -> u32 {
        0
    }
}

#[tokio::test]
async fn test_engine_multiple_runs_share_resources() {
    // 同一 Engine 多次 run 不同 Spider，各自独立 stats/items
    // 端口 1 不可达，请求会立即失败（connection refused）
    let engine = Engine::infra().max_pages(10).build().unwrap();

    let (stats_a, items_a) = engine
        .run(CountSpider {
            name: "a".into(),
            url: "http://127.0.0.1:1/".into(),
        })
        .await
        .unwrap();
    let (stats_b, items_b) = engine
        .run(CountSpider {
            name: "b".into(),
            url: "http://127.0.0.1:1/".into(),
        })
        .await
        .unwrap();

    // 不可达 URL 不产出 item，pages 不递增（请求失败不计入 pages_crawled）
    assert_eq!(items_a.len(), 0, "不可达 URL 不应产出 item");
    assert_eq!(items_b.len(), 0);
    assert_eq!(stats_a.pages_crawled, 0, "请求失败不计入 pages_crawled");
    assert_eq!(stats_b.pages_crawled, 0);
    // 两次 run 的 stats 完全隔离（各自独立 SpiderStats）
    assert_eq!(stats_a.errors, stats_b.errors);
}

#[tokio::test]
async fn test_engine_control_isolation() {
    // 两个 Engine 实例控制状态完全隔离（解决 I4）
    let engine_a = Engine::infra().build().unwrap();
    let engine_b = Engine::infra().build().unwrap();

    engine_a.control().pause_all();
    assert!(
        !engine_b.control().is_shutdown(),
        "Engine B 不应受 A 的 pause_all 影响"
    );

    engine_a.control().shutdown();
    assert!(engine_a.control().is_shutdown());
    assert!(
        !engine_b.control().is_shutdown(),
        "Engine B 不应受 A 关闭影响"
    );
}

#[tokio::test]
async fn test_engine_run_returns_items() {
    // 验证 run 返回 (stats, items)，items 被正确收集
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let html = "<p>item1</p><p>item2</p>";
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    html.len(),
                    html
                );
                let _ = socket.write_all(resp.as_bytes()).await;
            });
        }
    });
    let base = format!("http://{}", addr);

    struct PSpider {
        url: String,
    }
    #[async_trait]
    impl Spider for PSpider {
        fn name(&self) -> &str {
            "p"
        }
        fn start_urls(&self) -> Vec<String> {
            vec![self.url.clone()]
        }
        async fn parse(&self, resp: SpiderResponse) -> (Vec<Value>, Vec<SpiderRequest>) {
            let doc = resp.parse().unwrap();
            let items: Vec<Value> = doc
                .select("p")
                .iter()
                .map(|n| serde_json::json!({"text": n.text()}))
                .collect();
            (items, vec![])
        }
        fn obey_robots(&self) -> bool {
            false
        }
    }

    let engine = Engine::infra().max_pages(1).build().unwrap();
    let (stats, items) = engine.run(PSpider { url: base }).await.unwrap();
    assert!(stats.pages_crawled >= 1, "应爬取至少 1 页");
    assert_eq!(items.len(), 2, "应产出 2 个 item");
    assert_eq!(items[0]["text"], "item1");
    assert_eq!(items[1]["text"], "item2");
}

#[tokio::test]
async fn test_engine_control_reset_between_runs() {
    // 验证每次 run 开始时 control 被 reset（清除上次的 shutdown/pause 状态）
    let engine = Engine::infra().build().unwrap();

    // run 前 shutdown
    engine.control().shutdown();
    assert!(engine.control().is_shutdown());

    // run 会 reset control，shutdown 被清除，run 能正常完成
    let (_stats, _items) = engine
        .run(CountSpider {
            name: "x".into(),
            url: "http://127.0.0.1:1/".into(),
        })
        .await
        .unwrap();
    assert!(
        !engine.control().is_shutdown(),
        "run 开始时应 reset shutdown 状态"
    );
}

#[tokio::test]
async fn test_engine_shutdown_via_control() {
    // 验证 Engine::shutdown() 调用 control.shutdown()
    let engine = Engine::infra().build().unwrap();
    assert!(!engine.control().is_shutdown());
    engine.shutdown();
    assert!(engine.control().is_shutdown());
}
