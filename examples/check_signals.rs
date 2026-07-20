use patchright_rs::{Browser, LaunchOptions};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ud = std::env::temp_dir().join(format!("pr-gl-{}", std::process::id()));
    let browser = Browser::launch(LaunchOptions { headless: true, user_data_dir: Some(ud.clone()), ..Default::default() }).await?;
    let page = browser.new_page().await?;
    page.goto("about:blank").await?;
    // Check headless detection vectors from main world
    let html = r#"data:text/html,<script>
    const c = document.createElement('canvas');
    const gl = c.getContext('webgl');
    let vendor='N/A', renderer='N/A';
    if (gl) {
        const d = gl.getExtension('WEBGL_debug_renderer_info');
        if (d) { vendor = gl.getParameter(d.UNMASKED_VENDOR_WEBGL); renderer = gl.getParameter(d.UNMASKED_RENDERER_WEBGL); }
    }
    document.title = JSON.stringify({
        vendor, renderer,
        screenW: screen.width, screenH: screen.height,
        outerW: window.outerWidth, outerH: window.outerHeight,
        innerW: window.innerWidth, innerH: window.innerHeight,
        deviceMemory: navigator.deviceMemory,
        hwConcurrency: navigator.hardwareConcurrency,
        platform: navigator.platform,
        wd: typeof navigator.webdriver,
        plugins: navigator.plugins.length,
        languages: navigator.languages,
    });
    </script>"#;
    page.goto(html).await?;
    println!("{}", page.evaluate_as_string("document.title").await?);
    browser.close().await?;
    let _ = std::fs::remove_dir_all(&ud);
    Ok(())
}
