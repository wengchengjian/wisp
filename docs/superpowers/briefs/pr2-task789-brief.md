# PR2 Task 7+8+9: browser/stealth 边界清理（合并执行）

本 brief 合并 PR2 的 Task 7、8、9，因为它们都是 browser/stealth 模块的小型机械修改，独立且互不依赖。实施者按顺序执行，每个 task 单独提交。

## Task 7: browser/patches.rs 迁移到 stealth/patches.rs

**Files:**
- Create: `src/stealth/patches.rs`（从 `src/browser/patches.rs` 复制全部内容）
- Delete: `src/browser/patches.rs`
- Modify: `src/browser/mod.rs`（删除 `pub mod patches;` 声明）
- Modify: `src/stealth/mod.rs`（添加 `pub mod patches;`）
- Modify: `src/browser/page.rs`（引用从 `crate::browser::patches::` 改为 `crate::stealth::patches::`）

### 步骤

1. 读取 `src/browser/patches.rs` 全部内容
2. 创建 `src/stealth/patches.rs`，内容与 `src/browser/patches.rs` 完全相同
3. 修改 `src/browser/mod.rs`，删除 `pub mod patches;` 行（及其上方注释 `/// 反检测 JS 补丁注入。`）
4. 修改 `src/stealth/mod.rs`，添加 `pub mod patches;`（按字母序在 `human` 后、`turnstile` 前）
5. 修改 `src/browser/page.rs` 第 116-118 行，将 `crate::browser::patches::` 改为 `crate::stealth::patches::`
6. 删除 `src/browser/patches.rs` 文件
7. 运行 `cargo test --lib stealth::patches && cargo test --lib browser::page`
8. 提交：`git add src/stealth/patches.rs src/stealth/mod.rs src/browser/mod.rs src/browser/page.rs && git rm src/browser/patches.rs && git commit -m "refactor: browser/patches.rs 迁移到 stealth/patches.rs"`

## Task 8: build_stealth_args 拆为 build_common_args + build_stealth_extra_args

**Files:**
- Modify: `src/browser/launch.rs`（拆分 `build_stealth_args`，新增 `build_common_args` + `build_stealth_extra_args`）
- Test: `src/browser/launch.rs`（内联测试模块）

### 当前状态

`src/browser/launch.rs` 当前有：
- `build_default_args(options)` = `build_stealth_args(options)` 加 `--` 前缀
- `build_stealth_args(options)` 返回所有参数（通用 + stealth 专用混在一起）

### 步骤

1. 在 `src/browser/launch.rs` 测试模块添加 4 个新测试（见下方代码）
2. 运行测试验证失败（`build_common_args` / `build_stealth_extra_args` 未定义）
3. 重构 `build_stealth_args` 为组合 `build_common_args` + `build_stealth_extra_args`
4. 运行测试验证通过
5. 提交：`git add src/browser/launch.rs && git commit -m "refactor: build_stealth_args 拆为 build_common_args + build_stealth_extra_args"`

### 新增测试代码

在 `src/browser/launch.rs` 的 `#[cfg(test)] mod tests` 中添加：

```rust
    #[test]
    fn test_build_common_args_excludes_stealth_specific() {
        let opts = LaunchOptions::default();
        let args = build_common_args(&opts);
        // 通用参数应包含 no-first-run、disable-background-networking
        assert!(args.contains(&"no-first-run".to_string()));
        assert!(args.contains(&"disable-background-networking".to_string()));
        // 通用参数不应包含 stealth 专用参数
        assert!(!args.contains(&"disable-blink-features=AutomationControlled".to_string()));
    }

    #[test]
    fn test_build_stealth_extra_args_contains_stealth_specific() {
        let opts = LaunchOptions::default();
        let args = build_stealth_extra_args(&opts);
        assert!(args.contains(&"disable-blink-features=AutomationControlled".to_string()));
        // stealth_extra 不应包含通用参数
        assert!(!args.contains(&"no-first-run".to_string()));
    }

    #[test]
    fn test_build_default_args_combines_common_and_stealth() {
        let opts = LaunchOptions::default();
        let args = build_default_args(&opts);
        // 应同时包含通用参数和 stealth 专用参数
        assert!(args.iter().all(|a| a.starts_with("--")));
        assert!(args.iter().any(|a| a == "--no-first-run"));
        assert!(args.iter().any(|a| a == "--disable-blink-features=AutomationControlled"));
    }

    #[test]
    fn test_build_stealth_args_equals_common_plus_stealth_extra() {
        let opts = LaunchOptions::default();
        let common = build_common_args(&opts);
        let extra = build_stealth_extra_args(&opts);
        let combined = build_stealth_args(&opts);
        // combined 应等于 common + extra（顺序可能不同，但内容相同）
        let mut expected = common.clone();
        expected.extend(extra.clone());
        expected.sort();
        let mut actual = combined.clone();
        actual.sort();
        assert_eq!(expected, actual);
    }
```

### 重构后的函数实现

替换 `build_default_args` 和 `build_stealth_args`，新增 `build_common_args` 和 `build_stealth_extra_args`：

```rust
/// Build default Chrome launch arguments from options, with patches applied.
/// These args include the "--" prefix (for testing/verification).
///
/// ARCH: = build_common_args + build_stealth_extra_args，加 "--" 前缀。
#[must_use]
pub fn build_default_args(options: &LaunchOptions) -> Vec<String> {
    build_stealth_args(options)
        .iter()
        .map(|a| format!("--{a}"))
        .collect()
}

/// 构建完整启动参数（common + stealth_extra），无 "--" 前缀。
///
/// ARCH: 保留原 `build_stealth_args` 名字以兼容 `Browser::launch` 调用。
pub fn build_stealth_args(options: &LaunchOptions) -> Vec<String> {
    let mut args = build_common_args(options);
    args.extend(build_stealth_extra_args(options));
    args
}

/// 构建通用 Chrome 启动参数（不含 stealth 专用参数）。
///
/// ARCH: 通用参数对所有浏览器模式（Dynamic/Stealth）都需要。
/// 不包含 `disable-blink-features=AutomationControlled` 等 stealth 专用参数。
pub fn build_common_args(options: &LaunchOptions) -> Vec<String> {
    let mut args: Vec<String> = vec![
        // patchright chromiumSwitches (verified from source)
        "disable-field-trial-config".into(),
        "disable-background-networking".into(),
        "disable-background-timer-throttling".into(),
        "disable-backgrounding-occluded-windows".into(),
        "disable-breakpad".into(),
        "no-default-browser-check".into(),
        "disable-dev-shm-usage".into(),
        "disable-hang-monitor".into(),
        "disable-prompt-on-repost".into(),
        "disable-renderer-backgrounding".into(),
        "force-color-profile=srgb".into(),
        "no-first-run".into(),
        "password-store=basic".into(),
        "use-mock-keychain".into(),
        "no-service-autorun".into(),
        "export-tagged-pdf".into(),
        "disable-search-engine-choice-screen".into(),
        "disable-infobars".into(),
        "disable-sync".into(),
        // Disabled features (from patchright)
        "disable-features=AvoidUnnecessaryBeforeUnloadCheckSync,DestroyProfileOnBrowserClose,DialMediaRouteProvider,GlobalMediaControls,HttpsUpgrades,LensOverlay,MediaRouter,PaintHolding".into(),
    ];

    // NOTE: Do NOT add --no-sandbox (causes Chrome re-launch on Windows headed mode)
    // NOTE: Do NOT add --enable-automation, --disable-popup-blocking, etc.

    // Proxy
    if let Some(ref proxy) = options.proxy {
        args.push(format!("proxy-server={}", proxy.server));
        // Chrome --proxy-server 不支持内联认证；username/password 无法通过命令行传递。
        // 需通过 CDP Fetch.requestPaused 拦截 407 或扩展程序处理（当前未实现）。
        if proxy.username.is_some() || proxy.password.is_some() {
            tracing::warn!(
                "Browser proxy auth (username/password) is not supported via --proxy-server. \
                 The proxy will be used without authentication; expect 407 responses. \
                 To use authenticated proxies with browser mode, configure the proxy to \
                 whitelist the client IP or use an unauthenticated proxy."
            );
        }
    }

    // User-provided extra args (strip -- prefix if present)
    for arg in &options.args {
        let stripped = arg.strip_prefix("--").unwrap_or(arg);
        args.push(stripped.to_string());
    }

    args
}

/// 构建 stealth 专用额外参数。
///
/// ARCH: PR2 阶段无 cfg gate（stealth 模块仍总是编译）。
/// PR3 启用 feature gate 后，此函数加 `#[cfg(feature = "stealth")]`，
/// `build_default_args` 中的调用也加对应 cfg。
pub fn build_stealth_extra_args(_options: &LaunchOptions) -> Vec<String> {
    vec![
        // Core anti-detection flag
        "disable-blink-features=AutomationControlled".into(),
    ]
}
```

**注意**：原 `build_stealth_args` 中的 `disable-blink-features=AutomationControlled` 行要从通用参数中移除（移到 `build_stealth_extra_args`），`disable-features=...` 行保留在通用参数中。

## Task 9: Browser::launch 注释清理

**Files:**
- Modify: `src/browser/mod.rs`（模块注释 + `Browser::launch` doc comment）

### 步骤

1. 修改 `src/browser/mod.rs` 第 1 行模块注释：
   - 修改前：`//! Browser process management. Launches Chrome directly with stealth args.`
   - 修改后：`//! Browser process management. Launches Chrome directly with launch args.`

2. 修改 `src/browser/mod.rs` 中 `Browser::launch` 的 doc comment（约第 49 行）：
   - 修改前：
     ```rust
     /// Launch browser with anti-detection patches.
     ///
     /// # Errors
     ```
   - 修改后：
     ```rust
     /// Launch browser with given options.
     ///
     /// ARCH: Browser 是通用 CDP 层，不包含反检测逻辑。
     /// 反检测 JS 补丁由 `stealth::patches` 提供（PR2 已迁移）。
     ///
     /// # Errors
     ```

3. 运行 `cargo test --lib browser`
4. 提交：`git add src/browser/mod.rs && git commit -m "docs: 清理 Browser::launch 注释，移除 anti-detection 描述"`

## 最终验证（3 个 task 都完成后）

Run: `cargo build --lib && cargo test --lib`
Expected: 全部 PASS，无警告。

## Global Constraints

- 变量命名 snake_case
- 代码注释用中文
- 提交信息用中文（一行）
- TDD：先写测试再写实现（Task 8 需要先写测试）
- 不向后兼容，只考虑最优解
- 使用 `expect` 而非 `unwrap` 提供错误上下文
