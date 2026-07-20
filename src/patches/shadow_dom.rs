/// JavaScript that forces all shadow roots to be created as 'open'.
pub const SHADOW_DOM_PATCH_SCRIPT: &str = r#"
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
