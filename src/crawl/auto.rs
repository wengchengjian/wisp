//! Auto 妯″紡鏍稿績缁勪欢锛氶€夋嫨鍣ㄨ拷韪€乁RL 娉涘寲銆佽鍒欏紩鎿庛€佹嫤鎴娴嬨€?
//!
//! Auto 妯″紡娴佺▼锛?
//! 1. 鍖归厤鐢ㄦ埛瑙勫垯 鈫?鐩存帴鐢ㄦ寚瀹氭ā寮?
//! 2. 鍖归厤鑷姩娉涘寲缂撳瓨 鈫?鐩存帴鐢ㄧ紦瀛樻ā寮?
//! 3. 閮芥病鍛戒腑 鈫?HTTP 鎶撳彇 鈫?妫€娴嬫槸鍚﹂渶瑕佸崌绾?

use std::collections::{HashMap, HashSet};
use regex::Regex;
use crate::fetcher::FetchMode;
use crate::error::{WispError, Result};

// === 閫夋嫨鍣ㄨ拷韪櫒 ===

/// 杩借釜 parse() 涓墍鏈夐€夋嫨鍣ㄨ皟鐢ㄥ強鍖归厤鏁般€?
///
/// Auto 妯″紡涓嬶紝SpiderResponse 鐨?css()/xpath_auto() 浼氳嚜鍔ㄨ褰曞埌姝よ拷韪櫒銆?
/// parse() 缁撴潫鍚?Engine 妫€鏌ユ槸鍚︽湁閫夋嫨鍣ㄨ繑鍥?0 鑺傜偣銆?
#[derive(Debug, Default)]
pub struct SelectorTracker {
    /// (selector, match_count)
    records: Vec<(String, usize)>,
}

impl SelectorTracker {
    pub fn new() -> Self {
        Self { records: Vec::new() }
    }

    /// 璁板綍涓€娆￠€夋嫨鍣ㄨ皟鐢ㄣ€?
    pub fn record(&mut self, selector: &str, match_count: usize) {
        self.records.push((selector.to_string(), match_count));
    }

    /// 鏄惁鏈夐€夋嫨鍣ㄨ繑鍥?0 鑺傜偣锛堟帓闄ょ敤鎴峰彲閫夐」锛夈€?
    ///
    /// 杩斿洖 true 琛ㄧず闇€瑕佸崌绾у埌 Dynamic 妯″紡銆?
    pub fn needs_upgrade(&self, exclude: &HashSet<String>) -> bool {
        self.records.iter().any(|(sel, count)| {
            *count == 0 && !exclude.contains(sel.as_str())
        })
    }

    /// 鑾峰彇鎵€鏈夎褰曪紙璋冭瘯鐢級銆?
    pub fn records(&self) -> &[(String, usize)] {
        &self.records
    }

    /// 璁板綍鏁伴噺銆?
    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

// === URL 娉涘寲绠楁硶 ===

/// 灏嗗叿浣?URL 娉涘寲涓烘鍒欐ā鏉裤€?
///
/// # 绀轰緥
/// - `/products/123` 鈫?`/products/\d+`
/// - `/user/deadbeef-cafe-1234/posts` 鈫?`/user/[a-f0-9-]+/posts`
/// - `/page/2` 鈫?`/page/\d+`
/// - `/about` 鈫?`/about`锛堜笉鍙橈級
pub fn generalize_url(url: &str) -> String {
    let path = url::Url::parse(url)
        .map(|u| u.path().to_string())
        .unwrap_or_else(|_| url.to_string());

    let segments: Vec<String> = path.split('/')
        .map(|seg| {
            if seg.is_empty() {
                return String::new();
            }
            // 绾暟瀛?鈫?\d+
            if seg.chars().all(|c| c.is_ascii_digit()) {
                return r"\d+".to_string();
            }
            // UUID/鍝堝笇 鈫?[a-f0-9-]+
            if is_uuid_or_hash(seg) {
                return r"[a-f0-9-]+".to_string();
            }
            // 淇濈暀瀛楅潰閲忥紙杞箟姝ｅ垯鐗规畩瀛楃锛?
            regex::escape(seg)
        })
        .collect();

    segments.join("/")
}

/// 鍒ゆ柇瀛楃涓叉槸鍚﹀儚 UUID 鎴栧搱甯屽€笺€?
fn is_uuid_or_hash(s: &str) -> bool {
    s.len() >= 8
        && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        && s.contains(|c: char| c.is_ascii_digit())
}

// === 妯″紡瑙勫垯寮曟搸 ===

/// URL 妯″紡 鈫?鎶撳彇妯″紡鐨勮鍒欏紩鎿庛€?
///
/// 浼樺厛绾э細鐢ㄦ埛瑙勫垯 > 鑷姩瀛︿範瑙勫垯 > None锛堣蛋 Auto 妫€娴嬶級
pub struct ModeRuleEngine {
    /// 鐢ㄦ埛瀹氫箟鐨勮鍒欙紙浼樺厛绾ф渶楂橈紝鎸夋坊鍔犻『搴忓尮閰嶏級
    user_rules: Vec<(Regex, FetchMode)>,
    /// 鑷姩娉涘寲鐨勭紦瀛樿鍒欙紙杩愯鏃跺涔狅級
    auto_rules: Vec<(Regex, FetchMode)>,
}

impl ModeRuleEngine {
    pub fn new() -> Self {
        Self {
            user_rules: Vec::new(),
            auto_rules: Vec::new(),
        }
    }

    /// 鐢ㄦ埛娣诲姞瑙勫垯锛堜紭鍏堢骇鏈€楂橈級銆?
    pub fn add_user_rule(&mut self, pattern: &str, mode: FetchMode) -> Result<()> {
        let re = Regex::new(pattern)
            .map_err(|e| WispError::CdpError(format!("invalid auto_rule regex '{}': {}", pattern, e)))?;
        self.user_rules.push((re, mode));
        Ok(())
    }

    /// 鑷姩瀛︿範锛氬皢 URL 娉涘寲涓烘鍒欐ā鏉垮悗瀛樺叆銆?
    ///
    /// 濡傛灉鐩稿悓妯℃澘宸插瓨鍦ㄥ垯鏇存柊妯″紡銆?
    pub fn learn(&mut self, url: &str, mode: FetchMode) {
        let pattern = generalize_url(url);
        // 妫€鏌ユ槸鍚﹀凡鏈夌浉鍚屾ā鏉?
        if let Ok(re) = Regex::new(&pattern) {
            // 鏇存柊宸叉湁瑙勫垯
            for (existing_re, existing_mode) in &mut self.auto_rules {
                if existing_re.as_str() == re.as_str() {
                    *existing_mode = mode;
                    return;
                }
            }
            // 鏂板瑙勫垯
            self.auto_rules.push((re, mode));
        }
    }

    /// 鏌ヨ URL 搴斾娇鐢ㄧ殑妯″紡銆?
    ///
    /// 浼樺厛绾э細鐢ㄦ埛瑙勫垯 > 鑷姩瑙勫垯 > None
    pub fn resolve(&self, url: &str) -> Option<FetchMode> {
        // 鎻愬彇璺緞鐢ㄤ簬鍖归厤
        let path = url::Url::parse(url)
            .map(|u| u.path().to_string())
            .unwrap_or_else(|_| url.to_string());

        // 鐢ㄦ埛瑙勫垯浼樺厛
        for (re, mode) in &self.user_rules {
            if re.is_match(&path) || re.is_match(url) {
                return Some(*mode);
            }
        }
        // 鑷姩瀛︿範瑙勫垯
        for (re, mode) in &self.auto_rules {
            if re.is_match(&path) || re.is_match(url) {
                return Some(*mode);
            }
        }
        None
    }

    /// 鐢ㄦ埛瑙勫垯鏁伴噺銆?
    pub fn user_rule_count(&self) -> usize {
        self.user_rules.len()
    }

    /// 鑷姩瑙勫垯鏁伴噺銆?
    pub fn auto_rule_count(&self) -> usize {
        self.auto_rules.len()
    }
}

impl Default for ModeRuleEngine {
    fn default() -> Self {
        Self::new()
    }
}

// === 鎷︽埅妫€娴?===

/// 妫€娴?HTTP 鍝嶅簲鏄惁琚弽鐖嫤鎴€?
///
/// 妫€娴嬩俊鍙凤細
/// - 鐘舵€佺爜 403/429/503
/// - 鍝嶅簲浣撳惈 Cloudflare 鎸戞垬鐗瑰緛
/// - 鍝嶅簲澶村惈 cf-chl-* 鏍囪
pub fn is_blocked_response(status: u16, body: &[u8], headers: &HashMap<String, String>) -> bool {
    // 鐘舵€佺爜
    if matches!(status, 403 | 429 | 503) {
        return true;
    }
    // CF 鐗瑰緛锛堝嵆浣?200 涔熷彲鑳芥槸鎸戞垬椤碉級
    let text = String::from_utf8_lossy(body).to_lowercase();
    if text.contains("just a moment")
        || text.contains("cf-challenge")
        || text.contains("challenge-platform")
        || text.contains("attention required")
        || text.contains("access denied")
    {
        return true;
    }
    // CF 鍝嶅簲澶?
    headers.keys().any(|k| k.starts_with("cf-chl"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // === URL 娉涘寲娴嬭瘯 ===

    #[test]
    fn test_generalize_numeric_id() {
        assert_eq!(generalize_url("https://shop.com/products/123"), "/products/\\d+");
        assert_eq!(generalize_url("https://shop.com/page/2"), "/page/\\d+");
    }

    #[test]
    fn test_generalize_uuid() {
        assert_eq!(
            generalize_url("https://shop.com/item/deadbeef-cafe-1234-5678"),
            "/item/[a-f0-9-]+"
        );
    }

    #[test]
    fn test_generalize_static_path() {
        assert_eq!(generalize_url("https://shop.com/about"), "/about");
        assert_eq!(generalize_url("https://shop.com/products/list"), "/products/list");
    }

    #[test]
    fn test_generalize_mixed() {
        assert_eq!(generalize_url("https://shop.com/user/42/posts"), "/user/\\d+/posts");
    }

    #[test]
    fn test_generalize_root() {
        assert_eq!(generalize_url("https://shop.com/"), "/");
    }

    // === 瑙勫垯寮曟搸娴嬭瘯 ===

    #[test]
    fn test_user_rule_priority() {
        let mut engine = ModeRuleEngine::new();
        engine.add_user_rule(r"/api/.*", FetchMode::Http).unwrap();
        // 鑷姩瑙勫垯璇?/api/data 闇€瑕?Dynamic
        engine.learn("https://shop.com/api/data", FetchMode::Dynamic);

        // 鐢ㄦ埛瑙勫垯浼樺厛
        assert_eq!(engine.resolve("https://shop.com/api/data"), Some(FetchMode::Http));
    }

    #[test]
    fn test_auto_rule_matches_similar() {
        let mut engine = ModeRuleEngine::new();
        engine.learn("https://shop.com/products/1", FetchMode::Dynamic);

        // 鍚屾ā鏉?URL 搴斿懡涓?
        assert_eq!(engine.resolve("https://shop.com/products/2"), Some(FetchMode::Dynamic));
        assert_eq!(engine.resolve("https://shop.com/products/999"), Some(FetchMode::Dynamic));
    }

    #[test]
    fn test_no_rule_returns_none() {
        let engine = ModeRuleEngine::new();
        assert_eq!(engine.resolve("https://shop.com/unknown/page"), None);
    }

    #[test]
    fn test_learn_updates_existing() {
        let mut engine = ModeRuleEngine::new();
        engine.learn("https://shop.com/products/1", FetchMode::Dynamic);
        engine.learn("https://shop.com/products/2", FetchMode::Stealth); // 鏇存柊

        assert_eq!(engine.auto_rule_count(), 1); // 涓嶆柊澧烇紝鏇存柊
        assert_eq!(engine.resolve("https://shop.com/products/3"), Some(FetchMode::Stealth));
    }

    // === 閫夋嫨鍣ㄨ拷韪祴璇?===

    #[test]
    fn test_tracker_zero_match_triggers() {
        let mut tracker = SelectorTracker::new();
        tracker.record(".product-card", 0);
        tracker.record(".header", 1);

        assert!(tracker.needs_upgrade(&HashSet::new()));
    }

    #[test]
    fn test_tracker_exclude_respected() {
        let mut tracker = SelectorTracker::new();
        tracker.record(".cookie-banner", 0); // 琚帓闄?
        tracker.record(".product-card", 5);

        let mut exclude = HashSet::new();
        exclude.insert(".cookie-banner".to_string());

        assert!(!tracker.needs_upgrade(&exclude));
    }

    #[test]
    fn test_tracker_all_matched_no_upgrade() {
        let mut tracker = SelectorTracker::new();
        tracker.record(".product-card", 10);
        tracker.record(".price", 10);
        tracker.record("h1", 1);

        assert!(!tracker.needs_upgrade(&HashSet::new()));
    }

    // === 鎷︽埅妫€娴嬫祴璇?===

    #[test]
    fn test_blocked_403() {
        assert!(is_blocked_response(403, b"", &HashMap::new()));
        assert!(is_blocked_response(429, b"", &HashMap::new()));
        assert!(is_blocked_response(503, b"", &HashMap::new()));
    }

    #[test]
    fn test_blocked_cf_200() {
        let body = b"<html><title>Just a moment...</title></html>";
        assert!(is_blocked_response(200, body, &HashMap::new()));

        let body2 = b"<div id='cf-challenge-running'></div>";
        assert!(is_blocked_response(200, body2, &HashMap::new()));
    }

    #[test]
    fn test_normal_200_not_blocked() {
        let body = b"<html><body><h1>Hello World</h1></body></html>";
        assert!(!is_blocked_response(200, body, &HashMap::new()));
    }

    #[test]
    fn test_blocked_cf_header() {
        let mut headers = HashMap::new();
        headers.insert("cf-chl-bypass".to_string(), "1".to_string());
        assert!(is_blocked_response(200, b"normal", &headers));
    }
}
