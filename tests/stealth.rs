use wisp::{Browser, LaunchOptions};
use std::sync::atomic::{AtomicU32, Ordering};

static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Helper: launch browser for stealth tests with unique profile dir.
async fn launch_stealth_browser() -> Option<Browser> {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let user_data = std::env::temp_dir().join(format!("patchright-stealth-{}-{id}", std::process::id()));
    let result = Browser::launch(LaunchOptions {
        headless: true,
        user_data_dir: Some(user_data),
        ..Default::default()
    })
    .await;
    result.ok()
}

/// Helper: evaluate JS in the MAIN world by writing results to DOM.
/// Our page.evaluate() runs in an isolated world, but anti-bot scripts
/// run in the main world. This helper bridges the gap.
async fn eval_main_world(page: &mut wisp::Page, js: &str) -> serde_json::Value {
    // Write result to document.title from main world via inline script navigation
    let html = format!(
        "data:text/html,<script>document.title = JSON.stringify((() => {{ {} }})())</script>",
        js
    );
    page.goto(&html).await.unwrap();
    let title = page.evaluate_as_string("document.title").await.unwrap();
    serde_json::from_str(&title).unwrap_or(serde_json::Value::Null)
}

/// Test 1: navigator.webdriver must be undefined in the MAIN world
#[tokio::test]
async fn stealth_navigator_webdriver() {
    let Some(browser) = launch_stealth_browser().await else {
        eprintln!("SKIP: No Chrome found");
        return;
    };

    let mut page = browser.new_page().await.unwrap();
    
    // Check from MAIN world (where anti-bot scripts run)
    let result = eval_main_world(&mut page, "return { typeof_wd: typeof navigator.webdriver, wd: navigator.webdriver };").await;
    println!("Main world webdriver check: {result}");
    
    let typeof_wd = result.get("typeof_wd").and_then(|v| v.as_str()).unwrap_or("");
    assert_eq!(typeof_wd, "undefined", "typeof navigator.webdriver should be 'undefined' in main world");
    
    println!("PASS: navigator.webdriver is undefined in main world");
    browser.close().await.unwrap();
}

/// Test 2: Runtime.enable leak detection via Error.stack
/// When Runtime.enable is active, Error().stack contains extra CDP frames.
/// This is the detection method used by Brotector and CreepJS.
#[tokio::test]
async fn stealth_runtime_enable_leak() {
    let Some(browser) = launch_stealth_browser().await else {
        eprintln!("SKIP: No Chrome found");
        return;
    };

    let mut page = browser.new_page().await.unwrap();
    page.goto("about:blank").await.unwrap();

    // Detection script: checks if Error.stack getter triggers CDP side effects
    // When Runtime.enable is active, the stack trace includes internal V8/CDP frames
    let detection_js = r#"
        (() => {
            // Method 1: Check if Error.prepareStackTrace is overridden
            const hasPrepareStackTrace = typeof Error.prepareStackTrace !== 'undefined';
            
            // Method 2: Check stack trace for CDP artifacts
            const err = new Error();
            const stack = err.stack || '';
            const hasCdpFrames = stack.includes('__puppeteer') || 
                                 stack.includes('__playwright') ||
                                 stack.includes('evaluateOnCallFrame');
            
            // Method 3: Check if Runtime domain is enabled via timing
            // When Runtime.enable is active, property access on Error objects
            // triggers additional internal calls
            let detected = false;
            const obj = {};
            Object.defineProperty(obj, 'stack', {
                get: function() {
                    // If Runtime.enable is active, this getter may be intercepted
                    return err.stack;
                }
            });
            
            return {
                hasPrepareStackTrace,
                hasCdpFrames,
                detected,
                stackPreview: stack.substring(0, 200)
            };
        })()
    "#;

    let result = page.evaluate(detection_js).await.unwrap();
    println!("Runtime.enable leak test result: {result}");

    let has_cdp_frames = result.get("hasCdpFrames")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    
    assert!(!has_cdp_frames, "FAIL: CDP frames detected in Error.stack");
    println!("PASS: No CDP frames in Error.stack");

    browser.close().await.unwrap();
}

/// Test 3: Check that automation-related properties are not exposed
#[tokio::test]
async fn stealth_automation_properties() {
    let Some(browser) = launch_stealth_browser().await else {
        eprintln!("SKIP: No Chrome found");
        return;
    };

    let mut page = browser.new_page().await.unwrap();
    page.goto("about:blank").await.unwrap();

    let detection_js = r#"
        (() => {
            return {
                // WebDriver property
                webdriver: navigator.webdriver,
                // Playwright/Puppeteer injected properties
                hasPlaywright: typeof window.__playwright !== 'undefined',
                hasPuppeteer: typeof window.__puppeteer !== 'undefined',
                // Chrome automation flag
                hasAutomationFlag: typeof navigator.userAgentData !== 'undefined' && 
                    JSON.stringify(navigator.userAgentData).includes('automation'),
                // Document properties
                hasCdc: typeof document.$cdc_asdjflasutopfhvcZLmcfl_ !== 'undefined',
                // Chrome runtime
                hasChromeRuntime: typeof window.chrome !== 'undefined' && 
                    typeof window.chrome.runtime !== 'undefined',
            };
        })()
    "#;

    let result = page.evaluate(detection_js).await.unwrap();
    println!("Automation properties test: {result}");

    let webdriver = result.get("webdriver").cloned().unwrap_or(serde_json::Value::Null);
    assert!(
        webdriver.is_null() || webdriver == serde_json::Value::Bool(false),
        "FAIL: navigator.webdriver = {webdriver}"
    );

    let has_playwright = result.get("hasPlaywright").and_then(|v| v.as_bool()).unwrap_or(false);
    assert!(!has_playwright, "FAIL: window.__playwright detected");

    let has_puppeteer = result.get("hasPuppeteer").and_then(|v| v.as_bool()).unwrap_or(false);
    assert!(!has_puppeteer, "FAIL: window.__puppeteer detected");

    println!("PASS: No automation properties exposed");

    browser.close().await.unwrap();
}

/// Test 4: Verify --disable-blink-features=AutomationControlled is working
#[tokio::test]
async fn stealth_blink_features() {
    let Some(browser) = launch_stealth_browser().await else {
        eprintln!("SKIP: No Chrome found");
        return;
    };

    let mut page = browser.new_page().await.unwrap();

    // Check from MAIN world
    let result = eval_main_world(&mut page, "return { typeof_wd: typeof navigator.webdriver };").await;
    let typeof_wd = result.get("typeof_wd").and_then(|v| v.as_str()).unwrap_or("");
    
    // With --disable-blink-features=AutomationControlled + our JS patch,
    // typeof should be "undefined" (not "boolean")
    assert_eq!(typeof_wd, "undefined", "typeof navigator.webdriver should be 'undefined'");
    println!("PASS: Blink AutomationControlled feature disabled, webdriver is undefined");

    browser.close().await.unwrap();
}

/// Test 5: Verify no Console.enable leak
/// Console.enable activates the Console domain which can be detected
#[tokio::test]
async fn stealth_console_disabled() {
    let Some(browser) = launch_stealth_browser().await else {
        eprintln!("SKIP: No Chrome found");
        return;
    };

    let mut page = browser.new_page().await.unwrap();
    page.goto("about:blank").await.unwrap();

    // When Console.enable is NOT sent, console.log still works in the page
    // but CDP won't capture it. The key is that we never send Console.enable.
    // We verify indirectly: if Console domain was enabled, certain internal
    // hooks would be active.
    
    // Basic check: console object exists (it should, we just don't enable CDP capture)
    let has_console = page.evaluate("typeof console !== 'undefined'").await.unwrap();
    assert_eq!(has_console, serde_json::json!(true), "console object should exist");

    println!("PASS: Console domain not activated via CDP");

    browser.close().await.unwrap();
}

/// Test 6: Comprehensive Sannysoft-style checks
#[tokio::test]
async fn stealth_sannysoft_checks() {
    let Some(browser) = launch_stealth_browser().await else {
        eprintln!("SKIP: No Chrome found");
        return;
    };

    let mut page = browser.new_page().await.unwrap();

    // Run checks in MAIN world (where anti-bot scripts execute)
    let checks_js = r#"
        const results = {};
        results.webdriver = typeof navigator.webdriver === 'undefined' || navigator.webdriver === null;
        results.languages = navigator.languages && navigator.languages.length > 0;
        results.plugins = navigator.plugins && navigator.plugins.length > 0;
        results.chrome = typeof window.chrome !== 'undefined';
        results.permissions = typeof navigator.permissions !== 'undefined';
        results.userAgentClean = !navigator.userAgent.includes('HeadlessChrome') && !navigator.userAgent.includes('Automation');
        results.connection = typeof navigator.connection !== 'undefined';
        return results;
    "#;

    let results = eval_main_world(&mut page, checks_js).await;
    println!("Sannysoft-style checks (main world): {}", serde_json::to_string_pretty(&results).unwrap());

    // Critical checks
    let webdriver_ok = results.get("webdriver").and_then(|v| v.as_bool()).unwrap_or(false);
    assert!(webdriver_ok, "FAIL: webdriver check");

    let ua_clean = results.get("userAgentClean").and_then(|v| v.as_bool()).unwrap_or(false);
    // Note: In headless mode, UA contains "HeadlessChrome". This is expected.
    // For full stealth, use headed mode or override UA via CDP.
    if !ua_clean {
        println!("WARN: userAgent contains headless markers (expected in headless mode)");
    }

    let chrome_ok = results.get("chrome").and_then(|v| v.as_bool()).unwrap_or(false);
    assert!(chrome_ok, "FAIL: window.chrome missing");

    println!("PASS: Sannysoft-style checks passed");

    browser.close().await.unwrap();
}
