//! EngineBuilder 构造器子模块。

mod build;
mod methods;

use super::Engine;
use super::config::EngineConfig;
use crate::engine::EngineRuntimeDraft;

/// Engine 构造器（Builder 模式）。
///
/// 用户配置集中在 `config`，运行时资源集中在 `draft`。
pub struct EngineBuilder {
    pub(crate) config: EngineConfig,
    pub(crate) draft: EngineRuntimeDraft,
}

impl Engine {
    /// 创建 Engine builder（纯基础设施构造器）。
    pub fn infra() -> EngineBuilder {
        EngineBuilder {
            config: EngineConfig::default(),
            draft: EngineRuntimeDraft::new(),
        }
    }
}
