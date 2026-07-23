//! 小说爬虫示例：首页书籍列表 → 书籍详情章节列表 → 章节内容爬取。
//!
//! 演示 SpiderBuilder 多 callback 路由（三级页面流转）+ meta 传递 + Engine 运行。
//!
//! 运行：cargo run --example novel_crawler

use std::time::Duration;
use serde_json::json;
use wisp::crawl::{Engine, SpiderBuilder};
use wisp::crawl::stop::MaxPages;
use wisp::crawl::middleware::{UaRotationMiddleware, HeadersMiddleware, CookieChallengeMiddleware, JsonlWriterPipeline};
use wisp::fetcher::{FetchClientConfig, FetchMode};

/// 小说条目结构。
#[derive(Debug, Clone, serde::Serialize)]
struct NovelItem {
    /// 书名
    title: String,
    /// 作者
    author: String,
    /// 章节标题
    chapter_title: String,
    /// 章节序号
    chapter_index: usize,
    /// 正文内容
    content: String,
    /// 来源 URL
    url: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 初始化日志
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    let spider = SpiderBuilder::new("qishuxia")
        .start_urls(vec!["https://www.qishuxia.com/"])
        .delay(Duration::from_millis(500))
        .obey_robots(false)
        // 代理配置（通过 FetchClientConfig 统一注入 HTTP 与浏览器请求）
        .fetch_client_config(FetchClientConfig {
            proxy: Some("http://127.0.0.1:7897".into()),
            ..Default::default()
        })
        // Auto 模式：先尝试 HTTP，遇 403/CF 拦截自动升级 Stealth 浏览器模式
        .mode(FetchMode::Auto)
        // 中间件：每次请求自动轮换 User-Agent
        .middleware(UaRotationMiddleware::desktop())
        // 中间件：自动解决多步 Cookie 挑战（403 + Set-Cookie + JS 重定向）
        .middleware(CookieChallengeMiddleware::default())
        // 中间件：添加常规浏览器请求头，降低被反爬拦截的概率
        .middleware(HeadersMiddleware::new(vec![
            ("Accept".into(), "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8".into()),
            ("Accept-Language".into(), "zh-CN,zh;q=0.9,en;q=0.8".into()),
            ("Referer".into(), "https://www.qishuxia.com/".into()),
            ("Connection".into(), "keep-alive".into()),
            ("Upgrade-Insecure-Requests".into(), "1".into()),
        ]))
        // Pipeline：item 产出后自动追加写入 JSONL 文件
        .pipeline(JsonlWriterPipeline::new("novel_output.jsonl"))
        // === 第一级：首页 → 提取书籍列表 → follow 到 "detail" ===
        .on("default", |resp| async move {
            let doc = match resp.parse() {
                Ok(d) => d,
                Err(_) => return (vec![], vec![]),
            };

            // 诊断信息：帮助确认页面是否加载成功
            let title = doc.select_one("title").map(|n| n.text()).unwrap_or_default();
            println!("[首页] status={} title={:?} body_len={}", resp.status, title, resp.body.len());

            let mut follows = vec![];

            // 尝试多个常见小说站选择器（按优先级回退）
            let selectors = [
                ".txt-list li .s2 a",
                ".list2 ul li .name a",
                ".listmain dl dd a",
                "#list dd a",
                ".bookbox .bookname a",
                ".book-item .title a",
                ".novellist li a",
            ];

            let mut found_selector = "";
            for sel in &selectors {
                let links = doc.select(sel);
                if !links.is_empty() {
                    found_selector = sel;
                    for a in links.iter() {
                        if let Some(href) = a.attr("href") {
                            let book_title = a.text().trim().to_string();
                            if !book_title.is_empty() && !href.is_empty() {
                                if let Some(req) = resp.follow_meta(&href, json!({
                                    "title": book_title,
                                    "author": ""
                                })) {
                                    follows.push(req.with_callback("detail"));
                                }
                            }
                        }
                    }
                    break;
                }
            }

            if follows.is_empty() {
                // 未找到任何书籍，输出页面片段供调试
                let body_text = String::from_utf8_lossy(&resp.body);
                let snippet = &body_text[..body_text.len().min(500)];
                eprintln!("[首页] 未找到书籍链接！页面片段:\n{}", snippet);
            } else {
                println!("[首页] 使用选择器 {:?} 发现 {} 本书籍", found_selector, follows.len());
            }

            (vec![], follows)
        })
        // === 第二级：书籍详情 → 提取章节列表 → follow 到 "chapter" ===
        .on("detail", |resp| async move {
            let doc = match resp.parse() {
                Ok(d) => d,
                Err(_) => return (vec![], vec![]),
            };

            // 从 meta 获取书名
            let title = resp.request.meta.get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("未知")
                .to_string();

            // 尝试提取作者
            let author = doc.select_one(".txt ul:nth-child(1)")
                .map(|n| n.text().trim().to_string())
                .unwrap_or_default();

            let mut follows = vec![];

            // 章节列表（常见选择器）
            let chapters = doc.select(".list ul li .name a");
            for (idx, ch) in chapters.iter().enumerate() {
                if let Some(href) = ch.attr("href") {
                    let ch_title = ch.text().trim().to_string();
                    if !ch_title.is_empty() && !href.is_empty() {
                        if let Some(req) = resp.follow_meta(&href, json!({
                            "title": title,
                            "author": author,
                            "chapter_title": ch_title,
                            "chapter_index": idx
                        })) {
                            follows.push(req.with_callback("chapter"));
                        }
                    }
                }
            }

            println!("[详情] 《{}》 作者:{} 共 {} 章", title, author, follows.len());
            (vec![], follows)
        })
        // === 第三级：章节页 → 提取正文 → 组装 NovelItem ===
        .on("chapter", |resp| async move {
            let doc = match resp.parse() {
                Ok(d) => d,
                Err(_) => return (vec![], vec![]),
            };

            let meta = &resp.request.meta;
            let title = meta.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let author = meta.get("author").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let chapter_title = meta.get("chapter_title").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let chapter_index = meta.get("chapter_index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

            // 提取正文（常见小说内容选择器）
            let content = doc.select_one("#content, #chaptercontent, .content, .chapter-content, #BookText, .read-content")
                .map(|n| {
                    // 清理正文：去除广告文本、多余空行
                    n.text()
                        .lines()
                        .map(|l| l.trim())
                        .filter(|l| !l.is_empty())
                        .filter(|l| !l.contains("本章未完") && !l.contains("点击下一页"))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();

            if content.is_empty() {
                return (vec![], vec![]);
            }

            let item = NovelItem {
                title,
                author,
                chapter_title,
                chapter_index,
                content,
                url: resp.url.clone(),
            };

            println!("[章节] 《{}》第{}章 {} ({}字)",
                item.title, item.chapter_index + 1, item.chapter_title, item.content.len());

            (vec![serde_json::to_value(&item).unwrap_or_default()], vec![])
        })
        .until(MaxPages(200))
        .build();

    // 构建引擎并运行（代理已在 SpiderBuilder.fetch_client_config 中配置）
    let engine = Engine::infra()
        .max_concurrent(4)
        .max_pages(200)
        .build()?;

    println!("=== 开始爬取 qishuxia.com ===\n");
    let (stats, items) = engine.run(spider).await?;

    println!("\n=== 爬取完成 ===");
    println!("{}", stats.summary());
    println!("共获取 {} 个章节条目", items.len());

    // 输出前 3 条示例
    for (i, item) in items.iter().take(3).enumerate() {
        println!("\n--- 条目 {} ---", i + 1);
        println!("  书名: {}", item["title"]);
        println!("  作者: {}", item["author"]);
        println!("  章节: {}", item["chapter_title"]);
        let content = item["content"].as_str().unwrap_or("");
        println!("  内容预览: {}...", &content[..content.len().min(100)]);
    }

    // items 已经通过 JsonlWriterPipeline 自动写入文件，这里仅做统计展示
    if !items.is_empty() {
        println!("\n结果已通过 pipeline 保存到 novel_output.jsonl");
    }

    Ok(())
}
