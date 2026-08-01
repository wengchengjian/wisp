use super::*;
use url::Url;

fn make_url(s: &str) -> Url {
    Url::parse(s).expect("合法 URL")
}

fn make_cookie(name: &str, value: &str, domain: &str) -> Cookie {
    Cookie {
        name: name.into(),
        value: value.into(),
        domain: domain.into(),
        path: "/".into(),
        secure: false,
        http_only: false,
        same_site: Some("Lax".into()),
        expires: None,
    }
}

#[tokio::test]
async fn mock_set_and_get_cookie() {
    let jar = MockCookieJar::new();
    let url = make_url("https://example.com/path");
    jar.set(make_cookie("session", "abc123", "example.com"))
        .await;

    let cookies = jar.get(&url).await;
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].name, "session");
    assert_eq!(cookies[0].value, "abc123");
}

#[tokio::test]
async fn mock_set_replaces_same_name() {
    let jar = MockCookieJar::new();
    jar.set(make_cookie("session", "v1", "example.com")).await;
    jar.set(make_cookie("session", "v2", "example.com")).await;

    let url = make_url("https://example.com/");
    let cookies = jar.get(&url).await;
    assert_eq!(cookies.len(), 1, "同名 cookie 应被替换");
    assert_eq!(cookies[0].value, "v2");
}

#[tokio::test]
async fn mock_clear_removes_matching_domain() {
    let jar = MockCookieJar::new();
    jar.set(make_cookie("a", "1", "example.com")).await;
    jar.set(make_cookie("b", "2", "other.com")).await;

    let url = make_url("https://example.com/");
    jar.clear(&url).await;

    let cookies = jar.get(&url).await;
    assert!(cookies.is_empty(), "example.com 的 cookie 应被清除");
}

#[tokio::test]
async fn mock_header_returns_joined_string() {
    let jar = MockCookieJar::new();
    jar.set(make_cookie("a", "1", "example.com")).await;
    jar.set(make_cookie("b", "2", "example.com")).await;

    let url = make_url("https://example.com/");
    let header = jar.header(&url).await;
    assert!(header.is_some());
    let header = header.expect("非空");
    assert!(header.contains("a=1"));
    assert!(header.contains("b=2"));
    assert!(header.contains("; "));
}

#[tokio::test]
async fn mock_header_none_when_empty() {
    let jar = MockCookieJar::new();
    let url = make_url("https://example.com/");
    let header = jar.header(&url).await;
    assert!(header.is_none());
}

#[tokio::test]
async fn mock_domain_filter_excludes_other_domains() {
    let jar = MockCookieJar::new();
    jar.set(make_cookie("a", "1", "example.com")).await;
    jar.set(make_cookie("b", "2", "other.com")).await;

    let url = make_url("https://example.com/");
    let cookies = jar.get(&url).await;
    assert_eq!(cookies.len(), 1);
    assert_eq!(cookies[0].name, "a");
}

#[tokio::test]
async fn mock_domain_boundary_does_not_match_suffix_lookalike() {
    let jar = MockCookieJar::new();
    jar.set(make_cookie("a", "1", "example.com")).await;
    let url = make_url("https://notexample.com/");
    assert!(
        jar.get(&url).await.is_empty(),
        "notexample.com 不应匹配 example.com"
    );
}

#[test]
fn cookie_serialization_roundtrip() {
    let c = make_cookie("test", "val", "example.com");
    let json = serde_json::to_string(&c).expect("序列化");
    let deserialized: Cookie = serde_json::from_str(&json).expect("反序列化");
    assert_eq!(c, deserialized);
}
