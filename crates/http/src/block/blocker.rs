//! 域名拦截器实现。

use std::collections::HashSet;

use super::ad_domains::AD_DOMAINS;

/// 域名拦截器。
///
/// 维护一个被拦截域名的 HashSet，支持：
/// - 手动添加域名（含子域名自动匹配）
/// - 启用内置广告/追踪器域名黑名单
#[derive(Debug, Clone, Default)]
pub struct DomainBlocker {
    /// 被拦截的域名集合
    blocked: HashSet<String>,
    /// 是否已启用内置广告拦截
    ad_blocking_enabled: bool,
}

impl DomainBlocker {
    /// 创建空的域名拦截器。
    pub fn new() -> Self {
        Self {
            blocked: HashSet::new(),
            ad_blocking_enabled: false,
        }
    }

    /// 拦截指定域名及其所有子域名。
    ///
    /// 例如 `block_domain("ads.example.com")` 会拦截：
    /// - ads.example.com
    /// - sub.ads.example.com
    pub fn block_domain(&mut self, domain: &str) {
        self.blocked.insert(domain.to_lowercase());
    }

    /// 批量拦截多个域名。
    pub fn block_domains(&mut self, domains: &[&str]) {
        for d in domains {
            self.block_domain(d);
        }
    }

    /// 启用内置广告/追踪器域名拦截（~3500 域）。
    pub fn enable_ad_blocking(&mut self) {
        if !self.ad_blocking_enabled {
            for domain in AD_DOMAINS {
                self.blocked.insert(domain.to_string());
            }
            self.ad_blocking_enabled = true;
        }
    }

    /// 判断给定 URL 是否应被拦截。
    ///
    /// 匹配规则：URL 的域名等于 blocked 集合中的某项，或是其子域名。
    /// 使用 HashSet contains 查找（O(k)，k=域名标签数），避免 O(n) 全量遍历。
    pub fn should_block(&self, url: &str) -> bool {
        let host = match url::Url::parse(url) {
            Ok(u) => u.host_str().unwrap_or("").to_lowercase(),
            Err(_) => return false,
        };
        self.matches_host(&host)
    }

    /// 判断给定域名是否应被拦截（不解析 URL）。
    pub fn should_block_host(&self, host: &str) -> bool {
        self.matches_host(&host.to_lowercase())
    }

    /// 内部匹配：逐级剥离最左标签做 HashSet contains 查找。
    ///
    /// 例如 `a.b.ads.com` 依次检查 `a.b.ads.com` → `b.ads.com` → `ads.com` → `com`，
    /// 最多 k 次 O(1) 哈希查找（k = 标签数，通常 3~5），无堆分配。
    fn matches_host(&self, host: &str) -> bool {
        if self.blocked.contains(host) {
            return true;
        }
        let mut remaining = host;
        while let Some(pos) = remaining.find('.') {
            remaining = &remaining[pos + 1..];
            if self.blocked.contains(remaining) {
                return true;
            }
        }
        false
    }

    /// 被拦截域名数量。
    pub fn len(&self) -> usize {
        self.blocked.len()
    }

    /// 拦截列表是否为空。
    pub fn is_empty(&self) -> bool {
        self.blocked.is_empty()
    }

    /// 是否已启用广告拦截。
    pub fn is_ad_blocking_enabled(&self) -> bool {
        self.ad_blocking_enabled
    }

    /// 生成 Chrome CDP Fetch.enable 的 patterns 参数（用于浏览器模式拦截）。
    ///
    /// 返回适用于 `Fetch.enable` 的 urlPattern 列表。
    pub fn to_cdp_patterns(&self) -> Vec<serde_json::Value> {
        self.blocked
            .iter()
            .map(|domain| {
                serde_json::json!({
                    "urlPattern": format!("*://*.{}/*", domain),
                    "requestStage": "Request"
                })
            })
            .collect()
    }
}
