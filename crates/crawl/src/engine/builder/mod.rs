//! EngineBuilder 构造器子模块。

mod build;
mod methods;

use super::Engine;
use super::config::EngineConfig;
use crate::engine::EngineRuntimeDraft;
use std::sync::Arc;

/// Engine 构造器（Builder 模式）。
///
/// 用户配置集中在 `config`，运行时资源集中在 `draft`。
pub struct EngineBuilder {
    pub(crate) config: EngineConfig,
    pub(crate) draft: EngineRuntimeDraft,
}

impl Engine {
    /// 创建 Engine builder（纯基础设施构造器）。
    ///
    /// **默认启用 checkpoint（断点续爬）**：使用 `FileStore::default()`（`./wisp-data`），
    /// 每 `checkpoint_interval`（默认 100）页保存一次进度。断点续爬是长爬虫的基本可靠性
    /// 需求——中断后重启应从上次队列继续，而非重爬全部。不需要持久化时用
    /// [`EngineBuilder::no_checkpoint`] 关闭。
    pub fn infra() -> EngineBuilder {
        let mut builder = EngineBuilder {
            config: EngineConfig::default(),
            draft: EngineRuntimeDraft::new(),
        };
        builder.draft.checkpoint_store = Some(Arc::new(wisp_storage::FileStore::default()));
        builder
    }
}
