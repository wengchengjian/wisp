//! robots.txt 规则模型与文本解析。

/// 单域名的 robots.txt 规则
#[derive(Debug, Clone, Default)]
pub struct RobotsRules {
    /// Disallowed paths
    pub disallowed: Vec<String>,
    /// Crawl-delay 秒数（若存在）
    pub crawl_delay: Option<f64>,
    /// Request-rate (requests per second, 若存在)
    pub request_rate: Option<f64>,
}

impl RobotsRules {
    /// 规则是否为空（disallowed 空 + 无 crawl_delay + 无 request_rate）。
    pub fn is_empty_rules(&self) -> bool {
        self.disallowed.is_empty() && self.crawl_delay.is_none() && self.request_rate.is_none()
    }
}

fn robots_section_for_line(line: &str) -> Option<bool> {
    if !line.starts_with("User-agent:") {
        return None;
    }
    Some(line["User-agent:".len()..].trim() == "*")
}

fn apply_robots_directive(rules: &mut RobotsRules, line: &str) {
    if let Some(path) = line.strip_prefix("Disallow:") {
        let path = path.trim();
        if !path.is_empty() {
            rules.disallowed.push(path.to_string());
        }
        return;
    }
    if let Some(val) = line.strip_prefix("Crawl-delay:") {
        if let Ok(delay) = val.trim().parse::<f64>() {
            rules.crawl_delay = Some(delay);
        }
        return;
    }
    if let Some(val) = line.strip_prefix("Request-rate:") {
        apply_request_rate(rules, val.trim());
    }
}

fn apply_request_rate(rules: &mut RobotsRules, val: &str) {
    let Some(slash_pos) = val.find('/') else {
        return;
    };
    let n_str = &val[..slash_pos];
    let d_str = val[slash_pos + 1..]
        .split_whitespace()
        .next()
        .unwrap_or("1");
    if let (Ok(n), Ok(d)) = (n_str.parse::<f64>(), d_str.parse::<f64>()) {
        if n > 0.0 && d > 0.0 {
            rules.request_rate = Some(n / d);
        }
    }
}

/// 仅采集 `User-agent: *` 段下的指令，支持 RFC 9309 的 `Disallow`，以及
/// `Crawl-delay`（秒）和 `Request-rate`（`N/D` 格式，转换为每秒请求数 N/D）。
/// 非法数值被静默忽略。空行和以 `#` 开头的注释行被跳过。
pub fn parse_robots_text(text: &str) -> RobotsRules {
    let mut rules = RobotsRules::default();
    let mut in_our_section = false;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(star) = robots_section_for_line(line) {
            in_our_section = star;
            continue;
        }
        if in_our_section {
            apply_robots_directive(&mut rules, line);
        }
    }
    rules
}
