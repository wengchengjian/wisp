//! wreq 切换后的 fetch 模块测试。
//!
//! 验证：emulation 配置、builder 链式调用、Config 默认值。
//! 不发起实际网络请求（避免环境依赖）。

use wisp::http::{Client, ClientBuilder, Config};
use wreq_util::Profile;

#[test]
fn test_config_default_has_chrome136_emulation() {
    let config = Config::default();
    assert_eq!(config.emulation, Some(Profile::Chrome136));
}

#[test]
fn test_builder_emulation_override() {
    let builder = ClientBuilder::new().emulation(Profile::Firefox128);
    assert_eq!(builder.config_ref().emulation, Some(Profile::Firefox128));
}

#[test]
fn test_builder_no_emulation() {
    let builder = ClientBuilder::new().no_emulation();
    assert_eq!(builder.config_ref().emulation, None);
}

#[test]
fn test_builder_chain_emulation_and_timeout() {
    let builder = ClientBuilder::new()
        .emulation(Profile::Safari18)
        .timeout(std::time::Duration::from_secs(60))
        .user_agent("test-agent");
    let config = builder.config_ref();
    assert_eq!(config.emulation, Some(Profile::Safari18));
    assert_eq!(config.timeout, std::time::Duration::from_secs(60));
    assert_eq!(config.user_agent.as_deref(), Some("test-agent"));
}

#[test]
fn test_client_build_with_emulation() {
    let client = Client::builder()
        .emulation(Profile::Chrome136)
        .timeout(std::time::Duration::from_secs(10))
        .build();
    assert!(
        client.is_ok(),
        "client build with emulation should succeed: {:?}",
        client.err()
    );
}

#[test]
fn test_client_build_with_no_emulation() {
    let client = Client::builder().no_emulation().build();
    assert!(
        client.is_ok(),
        "client build without emulation should succeed: {:?}",
        client.err()
    );
}
