use chromiumoxide::Page as CdpPage;
use chromiumoxide::cdp::browser_protocol::page::AddScriptToEvaluateOnNewDocumentParams;

use crate::error::{PatchrightError, Result};

/// JavaScript that forces all shadow roots to be created as 'open'.
const SHADOW_DOM_PATCH_SCRIPT: &str = r#"
(() => {
    const originalAttachShadow = Element.prototype.attachShadow;
    Element.prototype.attachShadow = function(init) {
        if (init && init.mode === 'closed') {
            init = Object.assign({}, init, { mode: 'open' });
        }
        return originalAttachShadow.call(this, init);
    };
})();
"#;

/// Inject the shadow DOM patch so it runs before any page scripts.
pub async fn inject(page: &CdpPage) -> Result<()> {
    page.execute(AddScriptToEvaluateOnNewDocumentParams::new(SHADOW_DOM_PATCH_SCRIPT))
        .await
        .map_err(|e| PatchrightError::CdpError(format!("Shadow DOM patch injection: {e}")))?;
    Ok(())
}
