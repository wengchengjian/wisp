//! Stealth 领域契约：Turnstile 配置和挑战类型定义。
//!
//! 这些是纯配置/类型定义，下沉到 core 打破循环依赖。

/// Turnstile 解决器可调参数。
#[derive(Debug, Clone)]
pub struct TurnstileConfig {
    /// 首次点击前的被动等待（等 Turnstile widget 加载）。
    /// 默认 1200ms。调大可避免第一次点击落在未就绪控件上（预热点击），
    /// 减少总点击次数；网络慢可再调高，快可调低。
    pub passive_wait_ms: u64,
    /// 点击间隔（第一次点击失败后重试间隔）。
    /// 默认 2000ms。
    pub click_interval_ms: u64,
    /// 循环检测间隔（检查挑战是否通过）。
    /// 默认 200ms。调低可更快检测到 bypass。
    pub poll_interval_ms: u64,
    /// 鼠标移动步数（模拟人类轨迹）。
    /// 默认 5。调低更快但可能被检测。
    pub mouse_steps: u32,
    /// 每步鼠标移动延迟 (ms)。
    /// 默认 15ms。
    pub mouse_step_delay_ms: u64,
    /// 鼠标按下到释放的延迟 (ms)。
    /// 默认 60ms。
    pub click_hold_ms: u64,
    /// DOM 查询深度（pierce shadow DOM）。
    /// 默认 10。调低更快但可能找不到 iframe。
    pub dom_depth: u32,
    /// 是否启用拟人化点击（贝塞尔轨迹 + 随机步数/间隔/偏移）。
    /// 默认 false（快速机械化，适合低检测风险站点）。
    /// 开启后点击间隔、鼠标移动、按下停留均随机化，降低被 CF 行为检测的概率。
    pub human_mode: bool,
}

impl Default for TurnstileConfig {
    fn default() -> Self {
        Self {
            passive_wait_ms: 1200,
            click_interval_ms: 1500,
            poll_interval_ms: 100,
            mouse_steps: 3,
            mouse_step_delay_ms: 5,
            click_hold_ms: 30,
            dom_depth: 10,
            human_mode: false,
        }
    }
}

/// Type of Cloudflare challenge detected on the page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChallengeType {
    /// No challenge detected.
    None,
    /// JavaScript challenge (5-second shield / IUAM).
    JsChallenge,
    /// Cloudflare Turnstile widget.
    Turnstile,
    /// Managed challenge (Cloudflare decides which to show).
    ManagedChallenge,
}
