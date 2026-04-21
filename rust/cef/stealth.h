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
    // chrome.runtime: handled by POST_LOAD_FIXUP in renderer_linux.c
    // CEF replaces window.chrome via V8 bindings after OnContextCreated,
    // so we can't set it here. The fixup script in on_load_end + setTimeout
    // adds it after CEF's init completes.
    "if (!window.chrome) window.chrome = {};"

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
    "  configurable: false"
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
    "  configurable: false"
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
    "  configurable: false"
    "});"

    // ── 8. navigator.connection.rtt (BotD headless check) ─────
    // Headless Chrome reports rtt=0. Real Chrome reports non-zero.
    "(function() {"
    "  if (navigator.connection) {"
    "    var orig = navigator.connection;"
    "    if (orig.rtt === 0) {"
    "      Object.defineProperty(orig, 'rtt', {get: function() { return 100; }, configurable: false});"
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
    "  Object.defineProperty(window, 'outerWidth', {get: function() { return 1920; }, configurable: false});"
    "  Object.defineProperty(window, 'outerHeight', {get: function() { return 1080; }, configurable: false});"
    "}"
    "if (window.innerWidth === 0) {"
    "  Object.defineProperty(window, 'innerWidth', {get: function() { return 1920; }, configurable: false});"
    "  Object.defineProperty(window, 'innerHeight', {get: function() { return 1080; }, configurable: false});"
    "}"

    // ── 12. navigator.languages (BotD checks empty) ──────────
    "if (!navigator.languages || navigator.languages.length === 0) {"
    "  Object.defineProperty(navigator, 'languages', {"
    "    get: function() { return ['en-US', 'en']; },"
    "    configurable: false"
    "  });"
    "}"

    // ── 13. navigator.deviceMemory (should be > 0) ───────────
    "if (!navigator.deviceMemory || navigator.deviceMemory === 0) {"
    "  Object.defineProperty(navigator, 'deviceMemory', {"
    "    get: function() { return 8; },"
    "    configurable: false"
    "  });"
    "}"

    // ── 14. iframe.contentWindow consistency ──────────────────
    // Ensure srcdoc iframes have consistent chrome/self properties
    "(function() {"
    "  var origCreate = document.createElement;"
    "  // No override needed if contentWindow already works correctly."
    "  // CEF multi-process mode handles this natively."
    "})();"

    // ── 15. Notification.permission override ──────────────────
    // CEF may report 'denied' or throw. Override to 'default' (consistent
    // with permissions.query returning 'prompt' from patch #6).
    "(function() {"
    "  try {"
    "    if (typeof Notification === 'undefined') {"
    "      window.Notification = {permission: 'default'};"
    "    } else if (Notification.permission === 'denied') {"
    "      Object.defineProperty(Notification, 'permission', {"
    "        get: function() { return 'default'; }, configurable: false"
    "      });"
    "    }"
    "  } catch(e) {}"
    "})();"

    // ── 16. navigator.productSub (BotD checks this) ────────
    // Must be '20030107' for Chrome/Safari/Opera
    "(function() {"
    "  if (navigator.productSub !== '20030107') {"
    "    Object.defineProperty(navigator, 'productSub', {"
    "      get: function() { return '20030107'; }, configurable: false"
    "    });"
    "  }"
    "})();"

    // ── 17. navigator.plugins instanceof PluginArray ────────
    // BotD checks that plugins has correct prototype chain.
    // Our fake from #3 returns a plain array. Fix the prototype.
    "(function() {"
    "  try {"
    "    var desc = Object.getOwnPropertyDescriptor(navigator, 'plugins');"
    "    if (desc && desc.get) {"
    "      var origGet = desc.get;"
    "      Object.defineProperty(navigator, 'plugins', {"
    "        get: function() {"
    "          var p = origGet.call(this);"
    "          if (p && !(p instanceof PluginArray)) {"
    "            Object.setPrototypeOf(p, PluginArray.prototype);"
    "          }"
    "          return p;"
    "        }, configurable: false"
    "      });"
    "    }"
    "  } catch(e) {}"
    "})();"

    // ── 18. screen/window consistency for OSR ───────────────
    // Ensure screen dimensions match window dimensions
    "(function() {"
    "  if (screen.width === 0 || screen.height === 0) {"
    "    Object.defineProperty(screen, 'width', {get: function(){return 1920;}, configurable:true});"
    "    Object.defineProperty(screen, 'height', {get: function(){return 1080;}, configurable:true});"
    "    Object.defineProperty(screen, 'availWidth', {get: function(){return 1920;}, configurable:true});"
    "    Object.defineProperty(screen, 'availHeight', {get: function(){return 1040;}, configurable:true});"
    "    Object.defineProperty(screen, 'colorDepth', {get: function(){return 24;}, configurable:true});"
    "    Object.defineProperty(screen, 'pixelDepth', {get: function(){return 24;}, configurable:true});"
    "  }"
    "})();"

    // ── 19. Broken image dimensions (headless returns 0x0) ──
    // Real Chrome returns 16x16 for broken images. Override prototype.
    "(function() {"
    "  Object.defineProperty(HTMLImageElement.prototype, 'naturalWidth', {"
    "    get: function() {"
    "      var w = this.getAttribute('data-natural-width');"
    "      if (w) return parseInt(w);"
    "      return this.complete && this.src ? 16 : 0;"
    "    }, configurable: false"
    "  });"
    "  Object.defineProperty(HTMLImageElement.prototype, 'naturalHeight', {"
    "    get: function() {"
    "      var h = this.getAttribute('data-natural-height');"
    "      if (h) return parseInt(h);"
    "      return this.complete && this.src ? 16 : 0;"
    "    }, configurable: false"
    "  });"
    "})();"

    // ── 20. Remove any leftover automation globals ──────────
    "delete window._phantom;"
    "delete window.__nightmare;"
    "delete window._selenium;"
    "delete window.callPhantom;"
    "delete window._Recaptcha;"
    "delete window.emit;"
    "delete window.spawn;"
    "delete window.Buffer;"

    // ── 21. Timezone consistency with IP geolocation ─────────
    // Server timezone (Europe/Berlin) leaks via Intl.DateTimeFormat.
    // Override to match the residential IP's location.
    // Read WICK_TZ env var, default to America/Denver (US Mountain).
    "(function() {"
    "  var tz = 'America/Denver';" // TODO: derive from IP geolocation
    "  var origDTF = Intl.DateTimeFormat;"
    "  Intl.DateTimeFormat = function(locale, opts) {"
    "    opts = opts || {};"
    "    if (!opts.timeZone) opts.timeZone = tz;"
    "    return new origDTF(locale, opts);"
    "  };"
    "  Intl.DateTimeFormat.prototype = origDTF.prototype;"
    "  Intl.DateTimeFormat.supportedLocalesOf = origDTF.supportedLocalesOf;"
    "})();"

    // ── 22. outerWidth/outerHeight offset ──────────────────────
    // Real Chrome: outerHeight = innerHeight + ~85px (toolbar/titlebar)
    // OSR mode: both are identical, which is a headless signal.
    "if (window.outerHeight === window.innerHeight) {"
    "  Object.defineProperty(window, 'outerHeight', {"
    "    get: function() { return window.innerHeight + 85; },"
    "    configurable: false"
    "  });"
    "}"

    // ── Cleanup ──────────────────────────────────────────────
    "delete window.__markNative;"
;
