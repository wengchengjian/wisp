# Patchright-RS 设计规格

## 概述

用 Rust 原生实现 patchright 的核心能力：通过 CDP（Chrome DevTools Protocol）直接控制 Chromium 浏览器，在协议层消除自动化检测特征，使浏览器自动化操作对反爬系统不可见。

## 背景：Patchright 核心原理

Patchright 是 Playwright 的反检测增强版，通过以下补丁消除自动化特征：

| 补丁 | 原理 | 检测规避 |
|------|------|----------|
| Runtime.enable 泄露 | 不调用 `Runtime.enable`，改用 `Page.createIsolatedWorld` 创建隔离 ExecutionContext 执行 JS | 避免 Error.stack getter 检测 |
| Console.enable 泄露 | 完全不发送 `Console.enable`，禁用 Console API | 避免 CDP 协议层检测 |
| 命令行 Flag 泄露 | 移除 `--enable-automation` 等，添加 `--disable-blink-features=AutomationControlled` | 避免 navigator.webdriver=true |
| Closed Shadow Root | 注入 JS 将 closed shadow root 强制转为 open | 支持穿透 closed shadow DOM |

## 架构决策

- **实现路线**：直接 CDP 实现（不依赖 Playwright Node.js 驱动）
- **基础库**：基于 `chromiumoxide` crate 扩展（成熟的 Rust 异步 CDP 库）
- **API 范围**：最小可用集（启动、导航、JS 执行、元素操作、截图）
- **使用形式**：Rust 库 crate，提供 async API
- **异步运行时**：tokio
- **浏览器支持**：仅 Chromium 系（Chrome、Edge、Chromium）

## 项目结构

```
patchright-rs/
├── Cargo.toml
├── src/
│   ├── lib.rs              # crate 入口，re-export 公共 API
│   ├── config.rs           # LaunchOptions, ProxyConfig 等配置
│   ├── error.rs            # PatchrightError 统一错误类型
│   ├── browser/
│   │   ├── mod.rs          # Browser 结构体，生命周期管理
│   │   ├── launch.rs       # 浏览器进程启动 + 参数补丁
│   │   └── context.rs      # BrowserContext（隔离会话）
│   ├── page/
│   │   ├── mod.rs          # Page 结构体
│   │   ├── navigate.rs     # goto, go_back, reload
│   │   ├── evaluate.rs     # JS 执行（隔离 ExecutionContext）
│   │   └── screenshot.rs   # 截图
│   ├── element/
│   │   ├── mod.rs          # ElementHandle, 元素查找与操作
│   │   └── selector.rs     # CSS/XPath 选择器
│   ├── cdp/
│   │   ├── mod.rs          # CDP 连接管理
│   │   ├── filter.rs       # CDP 命令过滤/拦截层
│   │   └── session.rs      # CDP 会话管理
│   └── patches/
│       ├── mod.rs          # 补丁注册
│       ├── args.rs         # 启动参数补丁
│       ├── runtime.rs      # Runtime.enable 替代逻辑
│       └── shadow_dom.rs   # Closed Shadow Root 穿透
├── examples/
│   └── basic.rs            # 最小使用示例
└── tests/
    └── integration.rs      # 集成测试
```

## 依赖

```toml
[dependencies]
chromiumoxide = { version = "0.7", features = ["tokio-runtime"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
tracing = "0.1"
```

## 核心模块设计

### 1. 启动参数补丁 (`patches/args.rs`)

对应 patchright 的 Command Flags 补丁（`chromiumSwitches.js` 修改）。

```rust
/// 需要从默认参数中移除的 flags
const REMOVE_ARGS: &[&str] = &[
    "--enable-automation",
    "--disable-popup-blocking",
    "--disable-component-update",
    "--disable-default-apps",
    "--disable-extensions",
];

/// 需要添加的 flags
const ADD_ARGS: &[&str] = &[
    "--disable-blink-features=AutomationControlled",
];

pub fn patch_launch_args(args: &mut Vec<String>) {
    args.retain(|a| !REMOVE_ARGS.contains(&a.as_str()));
    for arg in ADD_ARGS {
        if !args.iter().any(|a| a == arg) {
            args.push(arg.to_string());
        }
    }
}
```

### 2. CDP 命令过滤层 (`cdp/filter.rs`)

拦截所有 CDP 命令，阻止泄露命令发送。对应 patchright 的 `crPagePatch.ts`、`crDevToolsPatch.ts`、`crServiceWorkerPatch.ts`。

```rust
/// 被阻止的 CDP 方法
const BLOCKED_METHODS: &[&str] = &[
    "Runtime.enable",
    "Console.enable",
];

pub fn should_block(method: &str) -> bool {
    BLOCKED_METHODS.contains(&method)
}
```

### 3. JS 执行 - 隔离 ExecutionContext (`page/evaluate.rs`)

对应 patchright 的 Runtime.enable 补丁核心逻辑。

```rust
async fn evaluate(&self, expression: &str) -> Result<serde_json::Value> {
    // 1. 创建隔离世界（不需要 Runtime.enable）
    let world = self.cdp.send(PageCreateIsolatedWorld {
        frame_id: self.frame_id.clone(),
        world_name: Some("patchright".into()),
        grant_universal_access: true,
    }).await?;

    // 2. 在隔离上下文中执行 JS
    let result = self.cdp.send(RuntimeEvaluate {
        expression: expression.to_string(),
        context_id: Some(world.execution_context_id),
        return_by_value: true,
        await_promise: true,
        ..Default::default()
    }).await?;

    if let Some(exception) = result.exception_details {
        return Err(PatchrightError::EvalError(exception.text));
    }

    Ok(result.result.value.unwrap_or(Value::Null))
}
```

### 4. Shadow DOM 穿透 (`patches/shadow_dom.rs`)

通过 `Page.addScriptToEvaluateOnNewDocument` 注入。

```rust
const SHADOW_DOM_SCRIPT: &str = r#"
(() => {
    const originalAttachShadow = Element.prototype.attachShadow;
    Element.prototype.attachShadow = function(init) {
        return originalAttachShadow.call(this, { ...init, mode: 'open' });
    };
})();
"#;

pub async fn inject_shadow_dom_patch(cdp: &CdpSession) -> Result<()> {
    cdp.send(PageAddScriptToEvaluateOnNewDocument {
        source: SHADOW_DOM_SCRIPT.to_string(),
        ..Default::default()
    }).await?;
    Ok(())
}
```

### 5. 浏览器启动 (`browser/launch.rs`)

```rust
pub async fn launch(options: LaunchOptions) -> Result<Browser> {
    // 1. 确定可执行文件路径
    let executable = resolve_executable(&options)?;

    // 2. 构造启动参数并应用补丁
    let mut args = build_default_args(&options);
    patches::args::patch_launch_args(&mut args);

    // 3. 启动进程
    let child = Command::new(&executable)
        .args(&args)
        .arg(format!("--remote-debugging-port={}", port))
        .spawn()?;

    // 4. 等待 CDP endpoint 就绪并连接
    let ws_url = wait_for_debugger_endpoint(port, options.timeout).await?;
    let cdp = CdpConnection::connect(&ws_url).await?;

    Ok(Browser { cdp, process: child, options })
}
```

## 用户 API

```rust
use patchright_rs::{Browser, LaunchOptions};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let browser = Browser::launch(LaunchOptions {
        headless: false,
        channel: Some("chrome".into()),
        user_data_dir: Some("./profile".into()),
        no_viewport: true,
        ..Default::default()
    }).await?;

    let page = browser.new_page().await?;
    page.goto("https://example.com").await?;

    // navigator.webdriver 应为 null（非 true）
    let webdriver = page.evaluate("navigator.webdriver").await?;
    assert!(webdriver.is_null());

    // 元素操作
    page.click("button.submit").await?;
    page.fill("input#search", "hello").await?;
    page.wait_for_selector(".result", Some(Duration::from_secs(5))).await?;

    // 截图
    page.screenshot("result.png").await?;

    browser.close().await?;
    Ok(())
}
```

## 配置项

```rust
pub struct LaunchOptions {
    pub headless: bool,                    // 默认 false
    pub channel: Option<String>,           // "chrome" | "msedge"
    pub executable_path: Option<PathBuf>,  // 自定义路径
    pub user_data_dir: Option<PathBuf>,    // 持久化数据
    pub no_viewport: bool,                 // 不固定视口
    pub args: Vec<String>,                 // 额外参数
    pub proxy: Option<ProxyConfig>,        // 代理
    pub timeout: Duration,                 // 启动超时，默认 30s
}

pub struct ProxyConfig {
    pub server: String,
    pub username: Option<String>,
    pub password: Option<String>,
}
```

## 错误处理

```rust
#[derive(Debug, thiserror::Error)]
pub enum PatchrightError {
    #[error("Browser launch failed: {0}")]
    LaunchFailed(String),

    #[error("CDP connection error: {0}")]
    CdpError(String),

    #[error("Navigation failed: {0}")]
    NavigationFailed(String),

    #[error("Element not found: {selector}")]
    ElementNotFound { selector: String },

    #[error("JS evaluation error: {0}")]
    EvalError(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, PatchrightError>;
```

## 行为一致性验证

| 验证项 | 方法 | 预期结果 |
|--------|------|----------|
| navigator.webdriver | `page.evaluate("navigator.webdriver")` | `null`（非 `true`） |
| 无 Runtime.enable | 监听 CDP WebSocket 流量 | 无 Runtime.enable 命令 |
| 无 Console.enable | 同上 | 无 Console.enable 命令 |
| 命令行参数 | 检查进程启动参数 | 无 --enable-automation，有 --disable-blink-features |
| console.log 不可用 | `page.evaluate("console.log('test')")` | 无输出（与 patchright 一致） |
| 检测网站通过 | 访问 sannysoft.com / browserscan.net | 不识别为机器人 |

## 分阶段实现

| 阶段 | 内容 | 交付物 |
|------|------|--------|
| Phase 1 | 项目骨架 + 浏览器启动 + CDP 连接 | 能启动 Chrome 并建立 CDP 连接 |
| Phase 2 | 启动参数补丁 + CDP 过滤层 | 无检测泄露的浏览器启动 |
| Phase 3 | 页面导航 + JS 执行（隔离上下文） | goto + evaluate 可用 |
| Phase 4 | 元素查找 + 点击/填写 + 等待 | 基本交互能力 |
| Phase 5 | 截图 + Shadow DOM 补丁 + 集成测试 | 完整最小可用集 |

## 假设与限制

1. **假设**：用户系统已安装 Chrome/Chromium/Edge，或可通过 `executable_path` 指定路径
2. **假设**：chromiumoxide 提供原始 CDP 命令发送接口（`execute()` 方法），允许我们绕过其高级 API 直接发送 CDP 命令。实现策略：不使用 chromiumoxide 的 `Page::evaluate()` 等可能内部发送 Runtime.enable 的高级方法，而是通过其底层 CDP 命令接口自行构造所有请求。如果 chromiumoxide 的底层接口不满足需求，则 fork 并修改
3. **限制**：仅支持 Chromium 系浏览器（与 patchright 一致）
4. **限制**：console.log 不可用（有意设计，与 patchright 一致）
5. **限制**：第一版不实现网络拦截、文件上传下载、多标签页管理
6. **风险**：chromiumoxide 内部可能在某些操作中自动发送 Runtime.enable，需要在集成时验证并处理
