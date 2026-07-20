use crate::error::Result;

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
///
/// Will be properly reimplemented in Task 5 with pipe-based CDP.
pub async fn inject(_session: &crate::cdp::session::CdpSession) -> Result<()> {
    todo!("Task 5: inject via pipe-based CDP Page.addScriptToEvaluateOnNewDocument")
}
