pub mod launch;

use chromiumoxide::browser::{Browser as CdpBrowser, BrowserConfig};
use futures::StreamExt;
use tokio::task::JoinHandle;

use crate::config::LaunchOptions;
use crate::error::{PatchrightError, Result};
use crate::page::Page;

/// A patched Chromium browser instance.
pub struct Browser {
    inner: CdpBrowser,
    handle: JoinHandle<()>,
    #[allow(dead_code)]
    options: LaunchOptions,
}

impl Browser {
    /// Launch a new browser instance with anti-detection patches applied.
    ///
    /// Key stealth measures:
    /// - Disables chromiumoxide's default args (which include --enable-automation)
    /// - Uses .hide() to add --disable-blink-features=AutomationControlled
    /// - Applies patchright launch arg patches
    /// - Never sends Runtime.enable or Console.enable (patched chromiumoxide)
    pub async fn launch(options: LaunchOptions) -> Result<Self> {
        let executable = launch::resolve_executable(&options)?;

        // Build our own stealth args (without -- prefix, chromiumoxide adds it)
        let stealth_args = launch::build_stealth_args(&options);

        let mut builder = BrowserConfig::builder()
            .chrome_executable(executable)
            // CRITICAL: disable chromiumoxide's default args which include --enable-automation
            .disable_default_args()
            // Add our stealth-friendly args
            .args(stealth_args)
            // Add --disable-blink-features=AutomationControlled
            .hide()
            .no_sandbox();

        // Set headless mode
        if options.headless {
            builder = builder.new_headless_mode();
        } else {
            builder = builder.with_head();
        }

        // Set user data dir if specified (avoids temp dir locking issues)
        if let Some(ref user_data_dir) = options.user_data_dir {
            builder = builder.user_data_dir(user_data_dir);
        }

        // Set launch timeout
        builder = builder.launch_timeout(options.timeout);

        let config = builder
            .build()
            .map_err(|e| PatchrightError::LaunchFailed(e.to_string()))?;

        let (inner, mut handler) = CdpBrowser::launch(config)
            .await
            .map_err(|e| PatchrightError::LaunchFailed(e.to_string()))?;

        // Spawn the handler in a background task
        let handle = tokio::spawn(async move {
            while let Some(_event) = handler.next().await {
                // Process events (drain them to keep connection alive)
            }
        });

        Ok(Self { inner, handle, options })
    }

    /// Create a new page (tab) in the browser.
    pub async fn new_page(&self) -> Result<Page> {
        let cdp_page = self
            .inner
            .new_page("about:blank")
            .await
            .map_err(|e| PatchrightError::CdpError(e.to_string()))?;

        Page::new(cdp_page).await
    }

    /// Close the browser and all its pages.
    pub async fn close(mut self) -> Result<()> {
        self.inner
            .close()
            .await
            .map_err(|e| PatchrightError::CdpError(e.to_string()))?;
        let _ = self.handle.await;
        Ok(())
    }
}
