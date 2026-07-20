use chromiumoxide::cdp::browser_protocol::page::AddScriptToEvaluateOnNewDocumentParams;
use chromiumoxide::Page as CdpPage;

use crate::error::{PatchrightError, Result};

/// JavaScript that hides all automation traces from the page.
/// Injected before any page scripts run via Page.addScriptToEvaluateOnNewDocument.
///
/// Patches:
/// 1. navigator.webdriver → undefined (at prototype level for robustness)
/// 2. chrome.runtime object (normal Chrome has it)
/// 3. navigator.plugins (non-empty for headed Chrome)
/// 4. navigator.permissions query fix
/// 5. navigator.languages fix
const STEALTH_SCRIPT: &str = r#"
(() => {
    // Patch 1: Override navigator.webdriver at PROTOTYPE level
    // This is more robust than instance-level override because detection scripts
    // may use Object.getOwnPropertyDescriptor(Navigator.prototype, 'webdriver')
    try {
        Object.defineProperty(Navigator.prototype, 'webdriver', {
            get: () => undefined,
            configurable: true,
        });
    } catch(e) {}

    // Also override at instance level as backup
    try {
        Object.defineProperty(navigator, 'webdriver', {
            get: () => undefined,
            configurable: true,
        });
    } catch(e) {}

    // Patch 2: Ensure chrome.runtime exists (normal Chrome has it)
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
    try {
        const originalQuery = window.navigator.permissions.query.bind(window.navigator.permissions);
        window.navigator.permissions.query = (parameters) => {
            if (parameters.name === 'notifications') {
                return Promise.resolve({ state: Notification.permission });
            }
            return originalQuery(parameters);
        };
    } catch(e) {}

    // Patch 4: Ensure plugins array looks normal
    // Headless Chrome has 0 plugins, real Chrome has at least a few.
    try {
        if (navigator.plugins.length === 0) {
            const makePlugin = (name, filename, description) => {
                const p = Object.create(Plugin.prototype);
                Object.defineProperties(p, {
                    name: { get: () => name, enumerable: true },
                    filename: { get: () => filename, enumerable: true },
                    description: { get: () => description, enumerable: true },
                    length: { get: () => 1, enumerable: true },
                });
                return p;
            };
            const plugins = [
                makePlugin('Chrome PDF Plugin', 'internal-pdf-viewer', 'Portable Document Format'),
                makePlugin('Chrome PDF Viewer', 'mhjfbmdgcfjbbpaeojofohoefgiehjai', ''),
                makePlugin('Native Client', 'internal-nacl-plugin', ''),
            ];
            Object.defineProperty(navigator, 'plugins', {
                get: () => {
                    const list = Object.create(PluginArray.prototype);
                    plugins.forEach((p, i) => {
                        Object.defineProperty(list, i, { get: () => p, enumerable: true });
                    });
                    Object.defineProperty(list, 'length', { get: () => plugins.length, enumerable: true });
                    list.item = (i) => plugins[i] || null;
                    list.namedItem = (n) => plugins.find(p => p.name === n) || null;
                    list.refresh = () => {};
                    list[Symbol.iterator] = function* () { yield* plugins; };
                    return list;
                },
                configurable: true,
            });
        }
    } catch(e) {}

    // Patch 5: Ensure languages are set
    try {
        if (!navigator.languages || navigator.languages.length === 0) {
            Object.defineProperty(navigator, 'languages', {
                get: () => Object.freeze(['en-US', 'en']),
                configurable: true,
            });
        }
    } catch(e) {}

    // Patch 6: Fix toString for overridden functions
    // Detection scripts may check Function.prototype.toString.call(navigator.permissions.query)
    // to see if it's been modified. We make our functions return native-looking strings.
    try {
        const nativeToString = Function.prototype.toString;
        const customToString = function() {
            if (this === window.navigator.permissions.query) {
                return 'function query() { [native code] }';
            }
            return nativeToString.call(this);
        };
        Function.prototype.toString = customToString;
    } catch(e) {}

    // Patch 7: Fix headless-specific WebGL detection
    // Headless Chrome uses SwiftShader which is a strong detection signal
    try {
        const getParameter = WebGLRenderingContext.prototype.getParameter;
        WebGLRenderingContext.prototype.getParameter = function(param) {
            // UNMASKED_VENDOR_WEBGL
            if (param === 37445) {
                return 'Google Inc. (NVIDIA)';
            }
            // UNMASKED_RENDERER_WEBGL
            if (param === 37446) {
                return 'ANGLE (NVIDIA, NVIDIA GeForce GTX 1080 Direct3D11 vs_5_0 ps_5_0, D3D11)';
            }
            return getParameter.call(this, param);
        };
        // Also patch WebGL2
        if (typeof WebGL2RenderingContext !== 'undefined') {
            const getParameter2 = WebGL2RenderingContext.prototype.getParameter;
            WebGL2RenderingContext.prototype.getParameter = function(param) {
                if (param === 37445) {
                    return 'Google Inc. (NVIDIA)';
                }
                if (param === 37446) {
                    return 'ANGLE (NVIDIA, NVIDIA GeForce GTX 1080 Direct3D11 vs_5_0 ps_5_0, D3D11)';
                }
                return getParameter2.call(this, param);
            };
        }
    } catch(e) {}

    // Patch 8: Ensure window.chrome.app exists
    try {
        if (window.chrome && !window.chrome.app) {
            window.chrome.app = {
                isInstalled: false,
                InstallState: { DISABLED: 'disabled', INSTALLED: 'installed', NOT_INSTALLED: 'not_installed' },
                RunningState: { CANNOT_RUN: 'cannot_run', READY_TO_RUN: 'ready_to_run', RUNNING: 'running' },
                getDetails: () => null,
                getIsInstalled: () => false,
            };
        }
    } catch(e) {}

    // Patch 9: Fix Notification.permission for headless
    // In headless Chrome, Notification.permission is 'denied' immediately
    try {
        if (typeof Notification !== 'undefined' && Notification.permission === 'denied') {
            Object.defineProperty(Notification, 'permission', {
                get: () => 'default',
                configurable: true,
            });
        }
    } catch(e) {}

    // Patch 10: Fix window dimensions for headless
    // Headless Chrome has outerHeight === innerHeight (no browser chrome)
    try {
        if (window.outerHeight === window.innerHeight) {
            Object.defineProperty(window, 'outerHeight', {
                get: () => window.innerHeight + 85,
                configurable: true,
            });
        }
        if (window.outerWidth === window.innerWidth) {
            Object.defineProperty(window, 'outerWidth', {
                get: () => window.innerWidth,
                configurable: true,
            });
        }
    } catch(e) {}

    // Patch 11: Fix screen dimensions for headless
    // Headless Chrome defaults to 800x600 screen which is a strong detection signal
    try {
        if (screen.width === 800 && screen.height === 600) {
            Object.defineProperty(screen, 'width', { get: () => 1920, configurable: true });
            Object.defineProperty(screen, 'height', { get: () => 1080, configurable: true });
            Object.defineProperty(screen, 'availWidth', { get: () => 1920, configurable: true });
            Object.defineProperty(screen, 'availHeight', { get: () => 1040, configurable: true });
            Object.defineProperty(screen, 'availLeft', { get: () => 0, configurable: true });
            Object.defineProperty(screen, 'availTop', { get: () => 0, configurable: true });
        }
    } catch(e) {}

    // Patch 12: Fix colorDepth/pixelDepth for headless
    try {
        if (screen.colorDepth !== 24) {
            Object.defineProperty(screen, 'colorDepth', { get: () => 24, configurable: true });
            Object.defineProperty(screen, 'pixelDepth', { get: () => 24, configurable: true });
        }
    } catch(e) {}
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
