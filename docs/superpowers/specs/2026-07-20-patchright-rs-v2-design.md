# Patchright-RS v2 设计规格：Pipe-based CDP + CLI

## 概述

重构 patchright-rs，将 CDP 通信从 WebSocket（chromiumoxide）改为 pipe-based（stdin/stdout），并通过 Browserscan 检测。同时添加完整的 CLI 工具。

## 动机

当前实现使用 chromiumoxide 的 WebSocket 通信（`--remote-debugging-port`），Browserscan 可检测到 CDP 连接。原版 patchright 使用 Playwright 的 pipe 通信（`--remote-debugging-pipe`），完全不可检测。

## Phase 1：Pipe-based CDP 客户端

### 通信协议

Chrome `--remote-debugging-pipe` 协议：
- 写入 Chrome stdin：`JSON_MESSAGE\0`
- 读取 Chrome stdout：`JSON_MESSAGE\0`
- 每条消息是完整 JSON 对象，以 null byte 分隔

### 模块结构

```
src/cdp/
├── mod.rs          # CdpSession 公共接口
├── pipe.rs         # PipeTransport（stdin/stdout 读写）
├── protocol.rs     # CdpMessage 类型定义
├── session.rs      # 会话管理（命令发送、响应路由、事件分发）
└── domains/
    ├── mod.rs
    ├── page.rs     # Page.navigate, Page.createIsolatedWorld, Page.captureScreenshot
    ├── runtime.rs  # Runtime.evaluate（带 contextId）
    ├── dom.rs      # DOM.querySelector, DOM.getDocument
    ├── target.rs   # Target.createTarget, Target.attachToTarget
    └── browser.rs  # Browser.close, Browser.getVersion
```

### PipeTransport

```rust
pub struct PipeTransport {
    writer: tokio::process::ChildStdin,
    reader: BufReader<tokio::process::ChildStdout>,
}

impl PipeTransport {
    pub async fn send(&mut self, msg: &serde_json::Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(msg)?;
        bytes.push(0); // null byte delimiter
        self.writer.write_all(&bytes).await?;
        self.writer.flush().await?;
        Ok(())
    }

    pub async fn recv(&mut self) -> Result<serde_json::Value> {
        let mut buf = Vec::new();
        self.reader.read_until(0, &mut buf).await?;
        buf.pop(); // remove trailing \0
        Ok(serde_json::from_slice(&buf)?)
    }
}
```

### CdpSession

```rust
pub struct CdpSession {
    transport: Arc<Mutex<PipeTransport>>,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>>,
    event_tx: broadcast::Sender<CdpEvent>,
}

impl CdpSession {
    /// 发送 CDP 命令并等待响应
    pub async fn execute(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        
        let msg = json!({ "id": id, "method": method, "params": params });
        self.transport.lock().await.send(&msg).await?;
        
        let response = rx.await?;
        if let Some(error) = response.get("error") {
            return Err(PatchrightError::CdpError(error.to_string()));
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    /// 启动后台读取循环
    pub fn spawn_reader(self: Arc<Self>) -> JoinHandle<()> { ... }
}
```

### 反检测关键约束

- **绝不发送** `Runtime.enable`
- **绝不发送** `Console.enable`
- JS 执行通过 `Page.createIsolatedWorld` → `Runtime.evaluate(contextId)` 实现
- 页面初始化只发送：`Page.enable`、`Page.getFrameTree`、`Page.setLifecycleEventsEnabled`

### 浏览器启动

```rust
pub async fn launch(options: LaunchOptions) -> Result<Browser> {
    let executable = resolve_executable(&options)?;
    let args = build_stealth_args(&options); // 保留现有补丁逻辑
    
    let mut child = Command::new(&executable)
        .args(&args)
        .arg("--remote-debugging-pipe")  // 关键：pipe 而非 port
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    
    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    
    let transport = PipeTransport::new(stdin, stdout);
    let session = CdpSession::new(transport);
    session.spawn_reader();
    
    Ok(Browser { session, process: child, options })
}
```

## Phase 2：Browser/Page API 层

### 公共 API（保持不变）

```rust
pub struct Browser { session: Arc<CdpSession>, process: Child, options: LaunchOptions }
pub struct Page { session: Arc<CdpSession>, frame_id: String }

impl Browser {
    pub async fn launch(options: LaunchOptions) -> Result<Self>;
    pub async fn new_page(&self) -> Result<Page>;
    pub async fn close(self) -> Result<()>;
}

impl Page {
    pub async fn goto(&self, url: &str) -> Result<()>;
    pub async fn evaluate(&self, expr: &str) -> Result<Value>;
    pub async fn evaluate_as_string(&self, expr: &str) -> Result<String>;
    pub async fn click(&self, selector: &str) -> Result<()>;
    pub async fn fill(&self, selector: &str, value: &str) -> Result<()>;
    pub async fn wait_for_selector(&self, selector: &str, timeout: Option<Duration>) -> Result<()>;
    pub async fn screenshot(&self, path: &str) -> Result<()>;
    pub async fn screenshot_bytes(&self) -> Result<Vec<u8>>;
}
```

### 页面创建流程

1. `Target.createTarget { url: "about:blank" }` → 获取 targetId
2. `Target.attachToTarget { targetId, flatten: true }` → 获取 sessionId
3. 在 session 上发送 `Page.enable`、`Page.setLifecycleEventsEnabled`
4. 注入 stealth 脚本（`Page.addScriptToEvaluateOnNewDocument`）
5. **不发送** `Runtime.enable`

### 反检测补丁（保留现有）

- `patches/args.rs`：启动参数补丁
- `patches/stealth.rs`：JS 注入（webdriver、WebGL、screen、plugins、toString）
- `patches/shadow_dom.rs`：Closed Shadow Root 穿透

## Phase 3：CLI 工具

### 命令

```
patchright install [chromium|chrome]     # 下载浏览器
patchright open <url>                    # 有头模式打开
patchright screenshot <url> <output>     # 截图
patchright pdf <url> <output>            # PDF
patchright run <script.js>               # 执行 JS 脚本
patchright codegen <url>                 # 录制生成代码
patchright --version                     # 版本
```

### 浏览器安装

- 下载源：Chrome for Testing CDN
- URL 格式：`https://storage.googleapis.com/chrome-for-testing-public/{version}/{platform}/chrome-{platform}.zip`
- 安装路径：`~/.patchright/browsers/chrome-{version}/`
- 版本记录：`~/.patchright/browsers.json`
- 平台标识：`win64`、`linux64`、`mac-arm64`、`mac-x64`

### 脚本执行（`patchright run`）

支持 JavaScript 自动化脚本，暴露 patchright API：

```javascript
// script.js
const page = await browser.newPage();
await page.goto('https://example.com');
await page.screenshot('out.png');
```

使用 `deno_core` 或 `boa` 作为 JS 运行时（或简单场景下直接通过 CDP `Runtime.evaluate` 执行）。

### 依赖

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
tracing = "0.1"
clap = { version = "4", features = ["derive"] }
reqwest = { version = "0.12", features = ["stream"] }  # 浏览器下载
zip = "2"                                              # 解压浏览器
base64 = "0.22"
futures = "0.3"

[[bin]]
name = "patchright"
path = "src/bin/patchright.rs"
```

## 移除项

- 完全移除 `chromiumoxide` 依赖
- 删除 `vendor/chromiumoxide/` 目录
- 删除 `src/cdp/filter.rs`（不再需要过滤层，因为根本不会发送被禁止的命令）

## 验证标准

1. `cargo test` 全部通过
2. Browserscan 总判定不为 "Robot"
3. Sannysoft webdriver 检测通过
4. `patchright install chrome` 成功下载
5. `patchright screenshot https://example.com out.png` 成功截图

## 假设与限制

1. Chrome `--remote-debugging-pipe` 在 Windows 上使用 stdin/stdout（已确认）
2. 第一版 CLI 的 `run` 命令仅支持通过 CDP 执行 JS（不嵌入完整 JS 运行时）
3. `codegen` 命令在第一版中为简单实现（监听 CDP 事件生成代码）
4. 仅支持 Chromium 系浏览器
