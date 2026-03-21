// wick Helper: CEF subprocess entry point with Chrome API stealth patches.
//
// This binary handles renderer, GPU, and other CEF subprocess types.
// For renderer subprocesses, it injects JavaScript in OnContextCreated
// to make CEF indistinguishable from real Chrome at the JS API level.

#include <stdio.h>
#include <string.h>

#import <Cocoa/Cocoa.h>

#include "include/capi/cef_app_capi.h"
#include "include/capi/cef_render_process_handler_capi.h"
#include "include/cef_api_hash.h"

// ── Ref counting ──────────────────────────────────────────────

static void CEF_CALLBACK add_ref(cef_base_ref_counted_t* s) { (void)s; }
static int CEF_CALLBACK release_fn(cef_base_ref_counted_t* s) { (void)s; return 1; }
static int CEF_CALLBACK has_one(cef_base_ref_counted_t* s) { (void)s; return 1; }
static int CEF_CALLBACK has_any(cef_base_ref_counted_t* s) { (void)s; return 1; }

static void init_base(cef_base_ref_counted_t* base, size_t size) {
    base->size = size;
    base->add_ref = add_ref;
    base->release = release_fn;
    base->has_one_ref = has_one;
    base->has_at_least_one_ref = has_any;
}

// ── Stealth JavaScript ────────────────────────────────────────
// Injected via OnContextCreated BEFORE any page scripts execute.
// Makes CEF's JS environment match real Chrome's.

static const char* STEALTH_JS =
    // ── Helper: make spoofed functions look native ────────────
    // Anti-spoofing detectors check toString() for [native code].
    // This wrapper makes our functions return the native signature.
    "(function() {"
    "  var nativeFn = Function.prototype.toString;"
    "  var fakeNatives = new WeakSet();"
    "  window.__markNative = function(fn) { fakeNatives.add(fn); return fn; };"
    "  Function.prototype.toString = function() {"
    "    if (fakeNatives.has(this)) return 'function ' + (this.name || '') + '() { [native code] }';"
    "    return nativeFn.call(this);"
    "  };"
    "  __markNative(Function.prototype.toString);"
    "})();"

    // ── 1. chrome.runtime (Cloudflare's primary check) ────────
    "if (!window.chrome) window.chrome = {};"
    "if (!window.chrome.runtime) {"
    "  window.chrome.runtime = {"
    "    OnInstalledReason: {CHROME_UPDATE:'chrome_update',INSTALL:'install',SHARED_MODULE_UPDATE:'shared_module_update',UPDATE:'update'},"
    "    OnRestartRequiredReason: {APP_UPDATE:'app_update',OS_UPDATE:'os_update',PERIODIC:'periodic'},"
    "    PlatformArch: {ARM:'arm',ARM64:'arm64',MIPS:'mips',MIPS64:'mips64',X86_32:'x86-32',X86_64:'x86-64'},"
    "    PlatformOs: {ANDROID:'android',CROS:'cros',LINUX:'linux',MAC:'mac',OPENBSD:'openbsd',WIN:'win'},"
    "    RequestUpdateCheckStatus: {NO_UPDATE:'no_update',THROTTLED:'throttled',UPDATE_AVAILABLE:'update_available'},"
    "    connect: __markNative(function connect() { return {onDisconnect:{addListener:function(){}},onMessage:{addListener:function(){}},postMessage:function(){}}; }),"
    "    sendMessage: __markNative(function sendMessage() {}),"
    "    id: undefined"
    "  };"
    "}"

    // ── 2. chrome.app (secondary Cloudflare/BotD check) ───────
    "if (!window.chrome.app) {"
    "  window.chrome.app = {"
    "    isInstalled: false,"
    "    InstallState: {DISABLED:'disabled',INSTALLED:'installed',NOT_INSTALLED:'not_installed'},"
    "    RunningState: {CANNOT_RUN:'cannot_run',READY_TO_RUN:'ready_to_run',RUNNING:'running'},"
    "    getDetails: __markNative(function getDetails() { return null; }),"
    "    getIsInstalled: __markNative(function getIsInstalled() { return false; }),"
    "    runningState: __markNative(function runningState() { return 'cannot_run'; })"
    "  };"
    "}"

    // ── 3. navigator.plugins (Cloudflare, Akamai) ─────────────
    "Object.defineProperty(navigator, 'plugins', {"
    "  get: __markNative(function plugins() {"
    "    var p = ["
    "      {name:'Chrome PDF Plugin',filename:'internal-pdf-viewer',description:'Portable Document Format',length:1,0:{type:'application/x-google-chrome-pdf',suffixes:'pdf',description:'Portable Document Format',enabledPlugin:null}},"
    "      {name:'Chrome PDF Viewer',filename:'mhjfbmdgcfjbbpaeojofohoefgiehjai',description:'',length:1,0:{type:'application/pdf',suffixes:'pdf',description:'',enabledPlugin:null}},"
    "      {name:'Native Client',filename:'internal-nacl-plugin',description:'',length:2,0:{type:'application/x-nacl',suffixes:'',description:'Native Client Executable',enabledPlugin:null},1:{type:'application/x-pnacl',suffixes:'',description:'Portable Native Client Executable',enabledPlugin:null}}"
    "    ];"
    "    p.item = function(i) { return this[i] || null; };"
    "    p.namedItem = function(n) { for(var i=0;i<this.length;i++) if(this[i].name===n) return this[i]; return null; };"
    "    p.refresh = function() {};"
    "    return p;"
    "  }),"
    "  configurable: true"
    "});"

    // ── 4. navigator.mimeTypes ────────────────────────────────
    "Object.defineProperty(navigator, 'mimeTypes', {"
    "  get: __markNative(function mimeTypes() {"
    "    var m = ["
    "      {type:'application/pdf',suffixes:'pdf',description:'',enabledPlugin:{name:'Chrome PDF Viewer'}},"
    "      {type:'application/x-google-chrome-pdf',suffixes:'pdf',description:'Portable Document Format',enabledPlugin:{name:'Chrome PDF Plugin'}},"
    "      {type:'application/x-nacl',suffixes:'',description:'Native Client Executable',enabledPlugin:{name:'Native Client'}},"
    "      {type:'application/x-pnacl',suffixes:'',description:'Portable Native Client Executable',enabledPlugin:{name:'Native Client'}}"
    "    ];"
    "    m.item = function(i) { return this[i] || null; };"
    "    m.namedItem = function(n) { for(var i=0;i<this.length;i++) if(this[i].type===n) return this[i]; return null; };"
    "    return m;"
    "  }),"
    "  configurable: true"
    "});"

    // ── 5. chrome.csi / chrome.loadTimes (legacy APIs) ────────
    "window.chrome.csi = __markNative(function csi() {"
    "  return {startE:Date.now(),onloadT:Date.now(),pageT:Date.now()/1000,tran:15};"
    "});"
    "window.chrome.loadTimes = __markNative(function loadTimes() {"
    "  return {"
    "    get requestTime(){return Date.now()/1000},"
    "    get startLoadTime(){return Date.now()/1000},"
    "    get commitLoadTime(){return Date.now()/1000},"
    "    get finishDocumentLoadTime(){return Date.now()/1000},"
    "    get finishLoadTime(){return Date.now()/1000},"
    "    get firstPaintTime(){return Date.now()/1000},"
    "    get firstPaintAfterLoadTime(){return 0},"
    "    get navigationType(){return 'Other'},"
    "    get wasFetchedViaSpdy(){return true},"
    "    get wasNpnNegotiated(){return true},"
    "    get npnNegotiatedProtocol(){return 'h2'},"
    "    get wasAlternateProtocolAvailable(){return false},"
    "    get connectionInfo(){return 'h2'}"
    "  };"
    "});"

    // ── 6. Permissions API consistency ────────────────────────
    "(function() {"
    "  var origQuery = navigator.permissions && navigator.permissions.query;"
    "  if (origQuery) {"
    "    navigator.permissions.query = __markNative(function query(desc) {"
    "      if (desc && desc.name === 'notifications') {"
    "        return Promise.resolve({state: Notification.permission, onchange: null});"
    "      }"
    "      return origQuery.apply(this, arguments);"
    "    });"
    "  }"
    "})();"

    // ── 7. navigator.webdriver = false ────────────────────────
    "Object.defineProperty(navigator, 'webdriver', {"
    "  get: __markNative(function webdriver() { return false; }),"
    "  configurable: true"
    "});"

    // ── 8. navigator.connection.rtt (BotD headless check) ─────
    // Headless Chrome reports rtt=0. Real Chrome reports non-zero.
    "(function() {"
    "  if (navigator.connection) {"
    "    var orig = navigator.connection;"
    "    if (orig.rtt === 0) {"
    "      Object.defineProperty(orig, 'rtt', {get: function() { return 100; }, configurable: true});"
    "    }"
    "  }"
    "})();"

    // ── 9. Remove CEF-specific globals (BotD checks these) ───
    "delete window.RunPerfTest;"
    "delete window.CefSharp;"
    "delete window.domAutomation;"
    "delete window.domAutomationController;"

    // ── 10. WebGL vendor/renderer spoofing ────────────────────
    // SwiftShader (software renderer) is an instant headless flag.
    // Spoof getParameter for UNMASKED_VENDOR/RENDERER constants.
    "(function() {"
    "  var origGetParam = WebGLRenderingContext.prototype.getParameter;"
    "  WebGLRenderingContext.prototype.getParameter = function(param) {"
    "    if (param === 37445) return 'Intel Inc.';"       // UNMASKED_VENDOR_WEBGL
    "    if (param === 37446) return 'Intel Iris OpenGL Engine';"  // UNMASKED_RENDERER_WEBGL
    "    return origGetParam.call(this, param);"
    "  };"
    "  __markNative(WebGLRenderingContext.prototype.getParameter);"
    "  if (typeof WebGL2RenderingContext !== 'undefined') {"
    "    var origGetParam2 = WebGL2RenderingContext.prototype.getParameter;"
    "    WebGL2RenderingContext.prototype.getParameter = function(param) {"
    "      if (param === 37445) return 'Intel Inc.';"
    "      if (param === 37446) return 'Intel Iris OpenGL Engine';"
    "      return origGetParam2.call(this, param);"
    "    };"
    "    __markNative(WebGL2RenderingContext.prototype.getParameter);"
    "  }"
    "})();"

    // ── 11. Window/screen dimensions for offscreen mode ───────
    "if (window.outerWidth === 0) {"
    "  Object.defineProperty(window, 'outerWidth', {get: function() { return 1920; }, configurable: true});"
    "  Object.defineProperty(window, 'outerHeight', {get: function() { return 1080; }, configurable: true});"
    "}"
    "if (window.innerWidth === 0) {"
    "  Object.defineProperty(window, 'innerWidth', {get: function() { return 1920; }, configurable: true});"
    "  Object.defineProperty(window, 'innerHeight', {get: function() { return 1080; }, configurable: true});"
    "}"

    // ── 12. navigator.languages (BotD checks empty) ──────────
    "if (!navigator.languages || navigator.languages.length === 0) {"
    "  Object.defineProperty(navigator, 'languages', {"
    "    get: function() { return ['en-US', 'en']; },"
    "    configurable: true"
    "  });"
    "}"

    // ── 13. navigator.deviceMemory (should be > 0) ───────────
    "if (!navigator.deviceMemory || navigator.deviceMemory === 0) {"
    "  Object.defineProperty(navigator, 'deviceMemory', {"
    "    get: function() { return 8; },"
    "    configurable: true"
    "  });"
    "}"

    // ── 14. iframe.contentWindow consistency ──────────────────
    // Ensure srcdoc iframes have consistent chrome/self properties
    "(function() {"
    "  var origCreate = document.createElement;"
    "  // No override needed if contentWindow already works correctly."
    "  // CEF multi-process mode handles this natively."
    "})();"

    // ── Cleanup ──────────────────────────────────────────────
    "delete window.__markNative;"
;

// ── Render process handler ────────────────────────────────────

static void CEF_CALLBACK on_context_created(
    cef_render_process_handler_t* self,
    cef_browser_t* browser,
    cef_frame_t* frame,
    struct _cef_v8_context_t* context) {
    (void)self; (void)browser; (void)context;

    // Inject stealth JS before any page script runs
    if (frame) {
        cef_string_t code = {};
        cef_string_utf8_to_utf16(STEALTH_JS, strlen(STEALTH_JS), &code);
        cef_string_t url = {};
        cef_string_utf8_to_utf16("", 0, &url);
        frame->execute_java_script(frame, &code, &url, 0);
        cef_string_clear(&code);
        cef_string_clear(&url);
    }
}

static cef_render_process_handler_t g_render_process_handler;

// ── App handler ───────────────────────────────────────────────

static cef_render_process_handler_t* CEF_CALLBACK get_render_process_handler(
    cef_app_t* self) {
    (void)self;
    return &g_render_process_handler;
}

static cef_app_t g_app;

// ── Main ──────────────────────────────────────────────────────

int main(int argc, char* argv[]) {
    @autoreleasepool {
        cef_api_hash(CEF_API_VERSION, 0);

        // Initialize the render process handler (stealth patches)
        init_base(&g_render_process_handler.base,
                   sizeof(cef_render_process_handler_t));
        g_render_process_handler.on_context_created = on_context_created;

        // Initialize the app handler
        init_base(&g_app.base, sizeof(cef_app_t));
        g_app.get_render_process_handler = get_render_process_handler;

        cef_main_args_t main_args = { .argc = argc, .argv = argv };
        int exit_code = cef_execute_process(&main_args, &g_app, NULL);
        return exit_code;
    }
}
