//! MCP 工具定义与输入参数 JSON Schema。

use serde_json::Value;
use std::pin::Pin;
use std::sync::LazyLock;

use crate::tools::ToolContext;
use wisp_core::error::Result;

/// 工具执行闭包：解析参数、调用 typed handler、输出 Value。
pub(crate) type ToolRun = Box<
    dyn for<'a> Fn(
            Value,
            &'a ToolContext<'a>,
        ) -> Pin<Box<dyn Future<Output = Result<Value>> + Send + 'a>>
        + Send
        + Sync,
>;

/// MCP 工具定义。
pub struct Tool {
    /// 工具名称。
    pub name: &'static str,
    /// 工具描述。
    pub description: &'static str,
    /// 输入参数 JSON Schema。
    pub input_schema: Value,
    run: ToolRun,
}

impl Tool {
    /// 创建工具 spec：元数据公开，执行逻辑收进同一模块。
    pub(crate) fn new(
        name: &'static str,
        description: &'static str,
        input_schema: Value,
        run: ToolRun,
    ) -> Self {
        Self {
            name,
            description,
            input_schema,
            run,
        }
    }

    /// 通过 spec 执行一次工具调用。
    pub(crate) async fn run(&self, args: Value, ctx: &ToolContext<'_>) -> Result<Value> {
        (self.run)(args, ctx).await
    }
}

/// 5 个工具覆盖核心场景；每个 spec 由对应工具模块提供。
pub static TOOLS: LazyLock<Vec<Tool>> = LazyLock::new(|| {
    vec![
        crate::tools::fetch::spec(),
        crate::tools::extract::spec(),
        crate::tools::crawl::spec(),
        crate::tools::adaptive::spec(),
        #[cfg(feature = "stealth")]
        crate::tools::stealth::spec(),
    ]
});
