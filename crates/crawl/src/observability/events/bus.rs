//! 事件总线：管理监听器并分发事件。

use super::{EngineEvent, EventListener};

/// 事件总线：管理监听器并分发事件。
#[derive(Clone)]
pub struct EventBus {
    listeners: Vec<EventListener>,
}

impl EventBus {
    /// 创建空事件总线。
    pub fn new() -> Self {
        Self {
            listeners: Vec::new(),
        }
    }

    /// 注册事件监听器。
    pub fn on(&mut self, listener: EventListener) {
        self.listeners.push(listener);
    }

    /// 发射事件（无 listener 时为 no-op）。
    pub async fn emit(&self, event: EngineEvent) {
        if self.listeners.is_empty() {
            return;
        }
        for listener in &self.listeners {
            listener(event.clone()).await;
        }
    }

    /// 是否有监听器。
    pub fn has_listeners(&self) -> bool {
        !self.listeners.is_empty()
    }

    /// 监听器数量。
    pub fn listener_count(&self) -> usize {
        self.listeners.len()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}
