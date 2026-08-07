//! 临时探测：验证 Auto 模式对 `https://www.bz444444444.com/` 能否成功降级。
//!
//! Auto 流程：首个请求走 HTTP → 被 CF 拦截 → StealthUpgrade 升级 Stealth 通过 →
//! cookie 写入共享 seam → 后续同域名请求命中 `try_http_with_cf_cookie` 快速路径，
//! 直接走 HTTP（降级），不再启动浏览器。
//!
//! 运行：
//! `cargo run --example bz_auto_downgrade_probe`
//!
//! 观察日志中是否出现：
//! - `StealthUpgrade: ... 升级 Stealth`（首请求被拦截升级）
//! - `AutoMode: HTTP+cookie 成功 (status=...), 跳过浏览器`（后续请求降级回 HTTP）

use std::time::Duration;
use wisp::{Engine, FetchClientConfig, FetchMode, SpiderBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    let transport = FetchClientConfig {
        // 升级到 Stealth 时必须真实渲染（headed + offscreen），否则 Turnstile iframe 不加载。
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
        .headers(vec![
            ("Accept".into(), "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8".into()),
            ("Accept-Language".into(), "zh-CN,zh;q=0.9,en;q=0.8".into()),
            ("Referer".into(), "https://www.bz444444444.com/".into()),
            ("Upgrade-Insecure-Requests".into(), "1".into()),
        ])
        .build()?;

    // 两个同域名 URL：第一个应触发 Stealth 升级，第二个应命中 HTTP+cookie 快速路径。
    let spider = SpiderBuilder::new("probe")
        .start_urls(vec![
            "https://www.bz444444444.com/".to_string(),
            "https://www.bz444444444.com/shuku/0-postdate-0-1.html".to_string(),
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
