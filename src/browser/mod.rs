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
    pub async fn launch(options: LaunchOptions) -> Result<Self> {
        let executable = launch::resolve_executable(&options)?;
        let args = launch::build_default_args(&options);

        // Strip "--" prefix from args for chromiumoxide builder
        let stripped_args: Vec<String> = args
            .iter()
            .map(|a| a.strip_prefix("--").unwrap_or(a).to_string())
            .collect();

        let mut builder = BrowserConfig::builder()
            .chrome_executable(executable)
            .args(stripped_args)
            .no_sandbox();

        // Set headless mode based on options
        if options.headless {
            builder = builder.new_headless_mode();
        } else {
            builder = builder.with_head();
        }

        // Set user data dir if specified
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
                // Process events (currently just draining them)
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
