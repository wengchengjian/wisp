//! MCP 工具定义与输入参数 JSON Schema。

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::pin::Pin;
use std::sync::LazyLock;

use crate::tools::{ToolContext, parse_args, to_value};
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

/// typed handler：接收已解析参数，返回 BoxFuture。
pub(crate) type TypedRun<A, O> =
    for<'a> fn(A, &'a ToolContext<'a>) -> Pin<Box<dyn Future<Output = Result<O>> + Send + 'a>>;

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
    /// 创建工具 spec：解析/序列化由本模块统一负责。
    pub(crate) fn from_handler<Args, Output>(
        name: &'static str,
        description: &'static str,
        input_schema: Value,
        run: TypedRun<Args, Output>,
    ) -> Self
    where
        Args: DeserializeOwned + Send + 'static,
        Output: Serialize + Send + 'static,
    {
        let run: ToolRun = Box::new(move |args, ctx| {
            let name = name;
            Box::pin(async move {
                let args = parse_args::<Args>(&args, name)?;
                let output = run(args, ctx).await?;
                to_value(output)
            })
        });
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

/// 核心工具集；每个 spec 由对应工具模块提供。
pub static TOOLS: LazyLock<Vec<Tool>> = LazyLock::new(|| {
    vec![
        crate::tools::fetch::spec(),
        crate::tools::extract::spec(),
        crate::tools::crawl::spec(),
        #[cfg(feature = "stealth")]
        crate::tools::stealth::spec(),
    ]
});
