//! Turnstile 解决器可调参数。

#[derive(Debug, Clone)]
pub struct TurnstileConfig {
    /// 首次点击前的被动等待（等 Turnstile widget 加载）。
    /// 默认 500ms。网络慢可调高，快可调低。
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
}

impl Default for TurnstileConfig {
    fn default() -> Self {
        Self {
            passive_wait_ms: 200,
            click_interval_ms: 1500,
            poll_interval_ms: 100,
            mouse_steps: 3,
            mouse_step_delay_ms: 5,
            click_hold_ms: 30,
            dom_depth: 10,
        }
    }
}
