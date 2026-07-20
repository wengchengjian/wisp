use patchright_rs::{Browser, LaunchOptions};
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let user_data = std::env::temp_dir().join(format!("pr-v4-{}", std::process::id()));
    let browser = Browser::launch(LaunchOptions { headless: true, user_data_dir: Some(user_data.clone()), ..Default::default() }).await?;
    let page = browser.new_page().await?;
    let html = r#"data:text/html,<script>document.title=JSON.stringify({wd:typeof navigator.webdriver,screenW:screen.width,screenH:screen.height,availW:screen.availWidth,availH:screen.availHeight,outerH:window.outerHeight,innerH:window.innerHeight,colorDepth:screen.colorDepth,webgl_vendor:(()=>{try{const c=document.createElement('canvas');const gl=c.getContext('webgl');const d=gl.getExtension('WEBGL_debug_renderer_info');return gl.getParameter(d.UNMASKED_VENDOR_WEBGL)}catch(e){return 'err'}})(),webgl_renderer:(()=>{try{const c=document.createElement('canvas');const gl=c.getContext('webgl');const d=gl.getExtension('WEBGL_debug_renderer_info');return gl.getParameter(d.UNMASKED_RENDERER_WEBGL)}catch(e){return 'err'}})()})</script>"#;
    page.goto(html).await?;
    println!("{}", page.evaluate_as_string("document.title").await?);
    browser.close().await?;
    let _ = std::fs::remove_dir_all(&user_data);
    Ok(())
}
