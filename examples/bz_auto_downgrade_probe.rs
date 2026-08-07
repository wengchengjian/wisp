//! 临时探测：验证 Auto 模式对 `https://www.bz444444444.com/` 能否成功降级。
//!
//! Auto 流程（规避 HTTP 前置污染）：首个请求**直接走 Stealth** 通过 CF 挑战 →
//! cookie 写入共享 seam → 后续同域名请求命中 `try_http_with_cf_cookie` 快速路径，
//! 直接走 HTTP（降级），不再启动浏览器。
//!
//! 运行：
//! `cargo run --example bz_auto_downgrade_probe`
//!
//! 观察日志中是否出现：
//! - `AutoMode: HTTP+cookie 成功 (status=200), 跳过浏览器`（后续请求降级回 HTTP）
//! - 首个请求直接走 Stealth 通过 CF 挑战，无 `StealthUpgrade` 升级日志

use std::time::Duration;
use wisp::{Engine, FetchClientConfig, FetchMode, SpiderBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let transport = FetchClientConfig {
        // force_headed_offscreen=true：有头 + offscreen。Stealth 通过 Turnstile 挑战
        // 依赖该配置（Turnstile iframe 加载 + CF 挑战绕过），headless 下挑战难通过。
        force_headed_offscreen: true,
        human_mode: true,
        challenge_timeout: Duration::from_secs(45),
        max_concurrent_pages: 1,
        ..Default::default()
    };

    let engine = Engine::infra()
        // Auto 模式：先 HTTP+cookie，被拦截时 StealthUpgrade 自动切 Stealth，
        // 通过后 cookie 复用，后续请求仍走 HTTP（降级）。
        .fetch_mode(FetchMode::Auto)
        .fetch_client_config(transport)
        .max_concurrent(1)
        .max_pages(20)
        .download_delay(Duration::from_millis(300))
        .obey_robots(false)
        .proxy("http://127.0.0.1:7897")
        .build()?;

    // 多个同域名 URL：第一个直接走 Stealth 通过 CF 挑战，其余命中 HTTP+cookie 快速路径
    //（降级后持续走 HTTP，不再反弹回 Stealth）。
    let spider = SpiderBuilder::new("probe")
        .start_urls(vec![
            "https://www.bz444444444.com/".to_string(),
            "https://www.bz444444444.com/shuku/0-postdate-0-1.html".to_string(),
            "https://www.bz444444444.com/shuku/0-postdate-0-2.html".to_string(),
            "https://www.bz444444444.com/shuku/0-postdate-0-3.html".to_string(),
            "https://www.bz444444444.com/shuku/0-postdate-0-4.html".to_string(),
            "https://www.bz444444444.com/shuku/0-postdate-0-5.html".to_string(),
        ])
        .on_page("default", |page| async move {
            let title = page
                .doc()
                .select("title")
                .first()
                .map(|n| n.text())
                .unwrap_or_default();
            println!(
                "PAGE  {:>3}  {}  (title={})",
                page.status(),
                page.url(),
                title.trim()
            );
            page
        })
        .build();

    println!("=== 开始验证 Auto 降级 bz444444444.com ===\n");
    let (_stats, items) = engine.run(spider).await?;
    println!("\n=== 完成，共抓取 {} 个页面 ===", items.len());
    Ok(())
}
