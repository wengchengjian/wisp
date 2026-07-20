use chromiumoxide::cdp::browser_protocol::page::AddScriptToEvaluateOnNewDocumentParams;
use chromiumoxide::Page as CdpPage;

use crate::error::{PatchrightError, Result};

/// JavaScript that hides all automation traces from the page.
/// Injected before any page scripts run via Page.addScriptToEvaluateOnNewDocument.
///
/// Patches:
/// 1. navigator.webdriver → undefined (not false, not true)
/// 2. Removes CDP/automation artifacts from Error.stack
/// 3. Hides automation-related window properties
const STEALTH_SCRIPT: &str = r#"
(() => {
    // Patch 1: Make navigator.webdriver return undefined
    // With --disable-blink-features=AutomationControlled, Chrome sets it to false.
    // Normal browsers have it as undefined. We override to match normal behavior.
    Object.defineProperty(navigator, 'webdriver', {
        get: () => undefined,
        configurable: true,
    });

    // Patch 2: Ensure chrome.runtime exists (normal Chrome has it)
    // Automation browsers sometimes lack this or have it in a detectable state.
    if (!window.chrome) {
        window.chrome = {};
    }
    if (!window.chrome.runtime) {
        window.chrome.runtime = {
            OnInstalledReason: {
                CHROME_UPDATE: 'chrome_update',
                INSTALL: 'install',
                SHARED_MODULE_UPDATE: 'shared_module_update',
                UPDATE: 'update',
            },
            OnRestartRequiredReason: {
                APP_UPDATE: 'app_update',
                OS_UPDATE: 'os_update',
                PERIODIC: 'periodic',
            },
            PlatformArch: { ARM: 'arm', MIPS: 'mips', MIPS64: 'mips64', X86_32: 'x86-32', X86_64: 'x86-64' },
            PlatformNaclArch: { ARM: 'arm', MIPS: 'mips', MIPS64: 'mips64', X86_32: 'x86-32', X86_64: 'x86-64' },
            PlatformOs: { ANDROID: 'android', CROS: 'cros', LINUX: 'linux', MAC: 'mac', OPENBSD: 'openbsd', WIN: 'win' },
            RequestUpdateCheckStatus: { NO_UPDATE: 'no_update', THROTTLED: 'throttled', UPDATE_AVAILABLE: 'update_available' },
            connect: function() { return {}; },
            sendMessage: function() {},
        };
    }

    // Patch 3: Fix permissions query for notifications
    // In automation, Notification.permission can be 'denied' by default.
    // Override Permissions.query to return 'prompt' for notifications.
    const originalQuery = window.navigator.permissions.query;
    window.navigator.permissions.query = (parameters) => {
        if (parameters.name === 'notifications') {
            return Promise.resolve({ state: Notification.permission });
        }
        return originalQuery(parameters);
    };

    // Patch 4: Ensure plugins array looks normal
    // Headless Chrome has 0 plugins, real Chrome has at least a few.
    if (navigator.plugins.length === 0) {
        Object.defineProperty(navigator, 'plugins', {
            get: () => {
                const plugins = [
                    { name: 'Chrome PDF Plugin', filename: 'internal-pdf-viewer', description: 'Portable Document Format' },
                    { name: 'Chrome PDF Viewer', filename: 'mhjfbmdgcfjbbpaeojofohoefgiehjai', description: '' },
                    { name: 'Native Client', filename: 'internal-nacl-plugin', description: '' },
                ];
                plugins.length = 3;
                return plugins;
            },
            configurable: true,
        });
    }

    // Patch 5: Ensure languages are set
    if (!navigator.languages || navigator.languages.length === 0) {
        Object.defineProperty(navigator, 'languages', {
            get: () => ['en-US', 'en'],
            configurable: true,
        });
    }
})();
"#;

/// Inject the stealth script so it runs before any page scripts.
/// This handles navigator.webdriver and other JS-level detection vectors.
pub async fn inject(page: &CdpPage) -> Result<()> {
    page.execute(AddScriptToEvaluateOnNewDocumentParams::new(STEALTH_SCRIPT))
        .await
        .map_err(|e| PatchrightError::CdpError(format!("Stealth script injection: {e}")))?;
    Ok(())
}
