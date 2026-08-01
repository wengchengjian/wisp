//! URL 模式 → 抓取模式的规则引擎。

use super::generalize_url;
use regex::Regex;
use wisp_core::error::{Result, WispError};
use wisp_fetcher::FetchMode;

const AUTO_RULE_LIMIT: usize = 1000;

/// URL 模式 → 抓取模式的规则引擎。
///
/// 优先级：用户规则 > 自动学习规则 > None（走 Auto 检测）
pub struct ModeRuleEngine {
    /// 用户定义的规则（优先级最高，按添加顺序匹配）
    user_rules: Vec<(Regex, FetchMode)>,
    /// 自动泛化的缓存规则（运行时学习）
    auto_rules: Vec<(Regex, FetchMode)>,
    /// 是否已发出 auto_rules 上限告警（只告警一次）。
    auto_rules_warned: bool,
}

impl ModeRuleEngine {
    /// 创建空的规则引擎。
    pub fn new() -> Self {
        Self {
            user_rules: Vec::new(),
            auto_rules: Vec::new(),
            auto_rules_warned: false,
        }
    }

    /// 用户添加规则（优先级最高）。
    pub fn add_user_rule(&mut self, pattern: &str, mode: FetchMode) -> Result<()> {
        let re = Regex::new(pattern).map_err(|e| {
            WispError::Parse(wisp_core::error::ParseError::Html(format!(
                "invalid auto_rule regex '{}': {}",
                pattern, e
            )))
        })?;
        self.user_rules.push((re, mode));
        Ok(())
    }

    /// 自动学习：将 URL 泛化为正则模板后存入。
    ///
    /// 如果相同模板已存在则更新模式。
    pub fn learn(&mut self, url: &str, mode: FetchMode) {
        let pattern = generalize_url(url);
        // 检查是否已有相同模板
        if let Ok(re) = Regex::new(&pattern) {
            // 更新已有规则
            for (existing_re, existing_mode) in &mut self.auto_rules {
                if existing_re.as_str() == re.as_str() {
                    *existing_mode = mode;
                    return;
                }
            }
            // 新增规则
            if self.auto_rules.len() >= AUTO_RULE_LIMIT {
                if !self.auto_rules_warned {
                    self.auto_rules_warned = true;
                    tracing::warn!("auto_rules 达到上限 {AUTO_RULE_LIMIT}，停止学习新规则");
                }
                return;
            }
            self.auto_rules.push((re, mode));
        }
    }

    /// 查询 URL 应使用的模式。
    ///
    /// 优先级：用户规则 > 自动规则 > None
    pub fn resolve(&self, url: &str) -> Option<FetchMode> {
        // 无规则时直接短路，避免为每个请求做 URL 解析。
        if self.user_rules.is_empty() && self.auto_rules.is_empty() {
            return None;
        }
        // 提取路径用于匹配
        let path = url::Url::parse(url)
            .map(|u| u.path().to_string())
            .unwrap_or_else(|_| url.to_string());

        // 用户规则优先
        for (re, mode) in &self.user_rules {
            if re.is_match(&path) || re.is_match(url) {
                return Some(*mode);
            }
        }
        // 自动学习规则
        for (re, mode) in &self.auto_rules {
            if re.is_match(&path) || re.is_match(url) {
                return Some(*mode);
            }
        }
        None
    }

    /// 用户规则数量。
    pub fn user_rule_count(&self) -> usize {
        self.user_rules.len()
    }

    /// 自动规则数量。
    pub fn auto_rule_count(&self) -> usize {
        self.auto_rules.len()
    }
}

impl Default for ModeRuleEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_fetcher::FetchMode;

    fn unique_literal_url(i: usize) -> String {
        let mut n = i;
        let mut letters = String::new();
        loop {
            letters.push((b'a' + (n % 26) as u8) as char);
            n /= 26;
            if n == 0 {
                break;
            }
        }
        format!("https://example.com/{letters}")
    }

    #[test]
    fn auto_rules_stop_learning_at_limit() {
        let mut engine = ModeRuleEngine::new();
        for i in 0..AUTO_RULE_LIMIT {
            engine.learn(&unique_literal_url(i), FetchMode::Http);
        }
        assert_eq!(engine.auto_rule_count(), AUTO_RULE_LIMIT);
        engine.learn("https://1.2.3.4/!!!", FetchMode::Stealth);
        assert_eq!(engine.auto_rule_count(), AUTO_RULE_LIMIT);
        assert_eq!(engine.resolve("https://1.2.3.4/!!!"), None);
    }
}
