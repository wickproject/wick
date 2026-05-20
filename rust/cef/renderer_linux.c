// wick-renderer (Linux): CEF offscreen renderer.
// Takes a URL as argv[1], renders via Chromium, outputs rendered HTML to stdout.
//
// On Linux, CEF runs in single-process mode without any window system.
// No X11/Wayland required — fully headless.
//
// To get React/SPA-rendered content (not just original HTML source),
// we wait for JS hydration, then use console.log(outerHTML) captured
// via CEF's display handler.

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <libgen.h>
#include <unistd.h>
#include <limits.h>
#include <time.h>

#include "stealth.h"
#include "include/capi/cef_app_capi.h"
#include "include/capi/cef_browser_capi.h"
#include "include/capi/cef_client_capi.h"
#include "include/capi/cef_display_handler_capi.h"
#include "include/capi/cef_frame_capi.h"
#include "include/capi/cef_life_span_handler_capi.h"
#include "include/capi/cef_load_handler_capi.h"
#include "include/capi/cef_render_handler_capi.h"
#include "include/capi/cef_string_visitor_capi.h"
#include "include/cef_api_hash.h"

// Protocol version marker — see renderer.m for usage and version history.
// Linux mirrors the macOS marker so the installer grep-check works
// identically across both platforms.
__attribute__((used)) static const char WICK_RENDERER_PROTOCOL[] =
    "WICK_RENDERER_PROTOCOL=v3-cf-aware";

// ── Globals ────────────────────────────────────────────────────

static cef_browser_t* g_browser = NULL;
static int g_load_done = 0;
static int g_source_done = 0;
static int g_click_x = -1;
static int g_click_y = -1;
static int g_clicked = 0;
static char g_click_prefix[48];
static char* g_html = NULL;
static size_t g_html_len = 0;
static char g_cookie[2048] = {}; // Optional cookie to inject (e.g., datadome=...)
// Random marker per process to avoid fingerprinting via console.log
static char g_marker[32];
// Optional CSS selector to wait for before dumping the DOM. Set per-render
// from the daemon stdin protocol (URL\tSELECTOR\n). Empty = no selector.
// See renderer.m for the macOS counterpart and full rationale.
static char g_selector[256] = {};

static void init_marker(void) {
    // Generate random hex strings from /dev/urandom
    FILE* f = fopen("/dev/urandom", "r");
    unsigned char buf[24];
    if (f && fread(buf, 1, sizeof(buf), f) == sizeof(buf)) {
        for (int i = 0; i < 12; i++)
            snprintf(g_marker + i*2, 3, "%02x", buf[i]);
        for (int i = 0; i < 12; i++)
            snprintf(g_click_prefix + i*2, 3, "%02x", buf[12+i]);
        fclose(f);
    } else {
        snprintf(g_marker, sizeof(g_marker), "_%d_%ld_", getpid(), (long)time(NULL));
        snprintf(g_click_prefix, sizeof(g_click_prefix), "_c%d_", getpid());
    }
}

// ── Ref counting (dummy — all handlers are static/global) ─────

static void CEF_CALLBACK add_ref(cef_base_ref_counted_t* self) { (void)self; }
static int CEF_CALLBACK release_fn(cef_base_ref_counted_t* self) { (void)self; return 1; }
static int CEF_CALLBACK has_one(cef_base_ref_counted_t* self) { (void)self; return 1; }
static int CEF_CALLBACK has_any(cef_base_ref_counted_t* self) { (void)self; return 1; }

static void init_base(cef_base_ref_counted_t* base, size_t size) {
    base->size = size;
    base->add_ref = add_ref;
    base->release = release_fn;
    base->has_one_ref = has_one;
    base->has_at_least_one_ref = has_any;
}

// ── Display handler — captures console.log output ─────────────

static int CEF_CALLBACK on_console_message(
    cef_display_handler_t* self, cef_browser_t* browser,
    cef_log_severity_t level, const cef_string_t* message,
    const cef_string_t* source, int line) {
    (void)self; (void)browser; (void)level; (void)source; (void)line;
    if (!message || !message->str || g_source_done) return 0;

    cef_string_utf8_t utf8 = {};
    cef_string_utf16_to_utf8(message->str, message->length, &utf8);

    // Check for click coordinates (Turnstile challenge detection)
    size_t cplen = strlen(g_click_prefix);
    if (utf8.length > cplen &&
        strncmp(utf8.str, g_click_prefix, cplen) == 0) {
        // Parse "x,y" coordinates
        int x = 0, y = 0;
        if (sscanf(utf8.str + cplen, "%d,%d", &x, &y) == 2) {
            g_click_x = x;
            g_click_y = y;
        }
        cef_string_utf8_clear(&utf8);
        return 1;  // suppress from console
    }

    // Check for our content marker prefix
    if (utf8.length > strlen(g_marker) &&
        strncmp(utf8.str, g_marker, strlen(g_marker)) == 0) {
        const char* html = utf8.str + strlen(g_marker);
        size_t html_len = utf8.length - strlen(g_marker);
        free(g_html);
        g_html = malloc(html_len);
        if (g_html) {
            memcpy(g_html, html, html_len);
            g_html_len = html_len;
        }
        g_source_done = 1;
    }

    cef_string_utf8_clear(&utf8);
    return 0;
}

static cef_display_handler_t g_display_handler;

// ── String visitor (fallback for non-SPA pages) ───────────────

static void CEF_CALLBACK visitor_visit(cef_string_visitor_t* self,
                                        const cef_string_t* string) {
    (void)self;
    if (g_source_done) return;  // Already got content via console
    if (string && string->str) {
        cef_string_utf8_t utf8 = {};
        cef_string_utf16_to_utf8(string->str, string->length, &utf8);
        fwrite(utf8.str, 1, utf8.length, stdout);
        cef_string_utf8_clear(&utf8);
    }
    fflush(stdout);
    g_source_done = 1;
}

static cef_string_visitor_t g_html_visitor;

// ── Load handler ──────────────────────────────────────────────

static int g_nav_count = 0;  // Track navigation count for WAF/challenge reloads

static void CEF_CALLBACK on_loading_state_change(
    cef_load_handler_t* self, cef_browser_t* browser,
    int isLoading, int canGoBack, int canGoForward) {
    (void)self; (void)canGoBack; (void)canGoForward;
    if (isLoading && browser) {
        // New navigation started (e.g., AWS WAF reload after challenge solve).
        // Reset state so content poller waits for the NEW page.
        g_nav_count++;
        g_load_done = 0;
        g_source_done = 0;
    }
    if (!isLoading && browser) {
        g_load_done = 1;
    }
}

static void CEF_CALLBACK on_load_start(cef_load_handler_t* self,
    cef_browser_t* browser, cef_frame_t* frame, cef_transition_type_t tt) {
    (void)self; (void)browser; (void)tt;
    if (frame && frame->is_main(frame)) {
        // Stealth patches (fallback for single-process mode)
        cef_string_t cef_js = {};
        cef_string_utf8_to_utf16(STEALTH_JS, strlen(STEALTH_JS), &cef_js);
        frame->execute_java_script(frame, &cef_js, NULL, 0);
        cef_string_clear(&cef_js);

        // Inject cookie if provided (for CAPTCHA retry after solving)
        if (g_cookie[0]) {
            char cookie_js[2200];
            snprintf(cookie_js, sizeof(cookie_js),
                "document.cookie='%s; path=/; secure; samesite=lax';",
                g_cookie);
            cef_string_t cjs = {};
            cef_string_utf8_to_utf16(cookie_js, strlen(cookie_js), &cjs);
            frame->execute_java_script(frame, &cjs, NULL, 0);
            cef_string_clear(&cjs);
        }
    }
}

// Minimal fixup for properties CEF resets after on_load_start.
// Do NOT re-run full STEALTH_JS — configurable:false prevents
// redefining properties, and the cascading errors kill everything.
static const char* POST_LOAD_FIXUP =
    "(function(){"
    // chrome.runtime: CEF creates chrome without runtime
    "  var rt={"
    "    OnInstalledReason:{CHROME_UPDATE:'chrome_update',INSTALL:'install',SHARED_MODULE_UPDATE:'shared_module_update',UPDATE:'update'},"
    "    OnRestartRequiredReason:{APP_UPDATE:'app_update',OS_UPDATE:'os_update',PERIODIC:'periodic'},"
    "    PlatformArch:{ARM:'arm',ARM64:'arm64',MIPS:'mips',MIPS64:'mips64',X86_32:'x86-32',X86_64:'x86-64'},"
    "    PlatformOs:{ANDROID:'android',CROS:'cros',LINUX:'linux',MAC:'mac',OPENBSD:'openbsd',WIN:'win'},"
    "    RequestUpdateCheckStatus:{NO_UPDATE:'no_update',THROTTLED:'throttled',UPDATE_AVAILABLE:'update_available'},"
    "    connect:function(){return{onDisconnect:{addListener:function(){}},onMessage:{addListener:function(){}},postMessage:function(){}};},"
    "    sendMessage:function(){},"
    "    id:undefined"
    "  };"
    "  function fix(){"
    "    if(window.chrome&&!window.chrome.runtime)window.chrome.runtime=rt;"
    "    if(window.outerHeight===window.innerHeight){"
    "      try{Object.defineProperty(window,'outerHeight',{get:function(){return window.innerHeight+85;},configurable:true});}catch(e){}"
    "    }"
    "  }"
    "  fix();"
    "  setTimeout(fix,0);"
    "  setTimeout(fix,1);"
    "  setTimeout(fix,10);"
    "  setTimeout(fix,50);"
    "})();";

static void CEF_CALLBACK on_load_end(cef_load_handler_t* self,
    cef_browser_t* browser, cef_frame_t* frame, int httpStatusCode) {
    (void)self; (void)browser; (void)httpStatusCode;
    if (frame && frame->is_main(frame)) {
        cef_string_t cef_js = {};
        cef_string_utf8_to_utf16(POST_LOAD_FIXUP, strlen(POST_LOAD_FIXUP), &cef_js);
        frame->execute_java_script(frame, &cef_js, NULL, 0);
        cef_string_clear(&cef_js);
    }
}

static void CEF_CALLBACK on_load_error(cef_load_handler_t* self,
    cef_browser_t* browser, cef_frame_t* frame,
    cef_errorcode_t errorCode, const cef_string_t* errorText,
    const cef_string_t* failedUrl) {
    (void)self; (void)browser; (void)errorText; (void)failedUrl;
    if (frame && frame->is_main(frame)) {
        fprintf(stderr, "wick-renderer: main frame load error %d\n", errorCode);
        g_source_done = 1;
    }
}

static cef_load_handler_t g_load_handler;

// ── Render handler (required for OSR, minimal no-op) ──────────

static void CEF_CALLBACK get_view_rect(cef_render_handler_t* self,
    cef_browser_t* browser, cef_rect_t* rect) {
    (void)self; (void)browser;
    rect->x = 0; rect->y = 0; rect->width = 1920; rect->height = 1080;
}

static void CEF_CALLBACK on_paint(cef_render_handler_t* self,
    cef_browser_t* browser, cef_paint_element_type_t type,
    size_t n, const cef_rect_t* d, const void* buf, int w, int h) {
    (void)self;(void)browser;(void)type;(void)n;(void)d;(void)buf;(void)w;(void)h;
}

static int CEF_CALLBACK get_screen_info(cef_render_handler_t* self,
    cef_browser_t* browser, cef_screen_info_t* info) {
    (void)self; (void)browser; (void)info; return 0;
}

static cef_render_handler_t g_render_handler;

// ── Life span handler ─────────────────────────────────────────

static void CEF_CALLBACK on_after_created(cef_life_span_handler_t* self,
    cef_browser_t* browser) {
    (void)self;
    g_browser = browser;
}

static int CEF_CALLBACK do_close(cef_life_span_handler_t* self,
    cef_browser_t* browser) {
    (void)self; (void)browser; return 0;
}

static void CEF_CALLBACK on_before_close(cef_life_span_handler_t* self,
    cef_browser_t* browser) {
    (void)self; (void)browser;
    g_source_done = 1;
}

static cef_life_span_handler_t g_life_span_handler;

// ── Client ────────────────────────────────────────────────────

static cef_life_span_handler_t* CEF_CALLBACK get_life_span_handler(cef_client_t* s) {
    (void)s; return &g_life_span_handler;
}
static cef_load_handler_t* CEF_CALLBACK get_load_handler(cef_client_t* s) {
    (void)s; return &g_load_handler;
}
static cef_render_handler_t* CEF_CALLBACK get_render_handler(cef_client_t* s) {
    (void)s; return &g_render_handler;
}
static cef_display_handler_t* CEF_CALLBACK get_display_handler(cef_client_t* s) {
    (void)s; return &g_display_handler;
}

static cef_client_t g_client;

// ── Get executable directory (Linux) ──────────────────────────

static int get_exe_dir(char* buf, size_t bufsize) {
    ssize_t len = readlink("/proc/self/exe", buf, bufsize - 1);
    if (len <= 0) return -1;
    buf[len] = '\0';
    char* dir = dirname(buf);
    if (dir != buf) {
        memmove(buf, dir, strlen(dir) + 1);
    }
    return 0;
}

// ── Main ──────────────────────────────────────────────────────

int main(int argc, char* argv[]) {
    const char* proxy_env = getenv("WICK_PROXY");
    char proxy_flag[256] = {};
    // Check if a display server is available (Xvfb or real X11)
    int has_display = (getenv("DISPLAY") != NULL);

    int extra_args = has_display ? 5 : 7;
    if (proxy_env && proxy_env[0]) {
        snprintf(proxy_flag, sizeof(proxy_flag), "--proxy-server=%s", proxy_env);
        extra_args++;
    }

    int new_argc = argc + extra_args;
    char** new_argv = malloc(sizeof(char*) * (new_argc + 1));
    for (int i = 0; i < argc; i++) new_argv[i] = argv[i];

    int a = argc;
    if (has_display) {
        // With Xvfb/X11: use software GL for WebGL support.
        // --ignore-gpu-blocklist forces WebGL even with software rendering
        // (otherwise getContext("webgl") returns null due to perf caveat).
        new_argv[a++] = "--no-sandbox";
        new_argv[a++] = "--use-fake-ui-for-media-stream";
        new_argv[a++] = "--in-process-gpu";
        new_argv[a++] = "--disable-gpu-compositing";
        new_argv[a++] = "--ignore-gpu-blocklist";
    } else {
        // Headless (no display): disable GPU, no WebGL available
        new_argv[a++] = "--disable-gpu";
        new_argv[a++] = "--disable-gpu-compositing";
        new_argv[a++] = "--no-sandbox";
        new_argv[a++] = "--ozone-platform=headless";
        new_argv[a++] = "--use-fake-ui-for-media-stream";
        new_argv[a++] = "--in-process-gpu";
        new_argv[a++] = "--enable-webgl";
    }
    if (proxy_env && proxy_env[0]) {
        new_argv[a++] = proxy_flag;
    }
    new_argv[a] = NULL;
    new_argc = a;

    init_marker();
    cef_api_hash(CEF_API_VERSION, 0);

    cef_main_args_t main_args = { .argc = new_argc, .argv = new_argv };
    int exit_code = cef_execute_process(&main_args, NULL, NULL);
    if (exit_code >= 0) {
        free(new_argv);
        return exit_code;
    }

    // Parse arguments: one-shot mode (URL as arg) or daemon mode (no URL)
    int one_shot = 0;
    const char* url_arg = NULL;
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--cookie") == 0 && i + 1 < argc) {
            snprintf(g_cookie, sizeof(g_cookie), "%s", argv[++i]);
        } else if (argv[i][0] != '-') {
            url_arg = argv[i];
            one_shot = 1;
        }
    }
    if (!one_shot && !url_arg) {
        // Daemon mode — will read URLs from stdin after init
    }

    // Initialize all handler structs
    init_base(&g_html_visitor.base, sizeof(cef_string_visitor_t));
    g_html_visitor.visit = visitor_visit;

    init_base(&g_display_handler.base, sizeof(cef_display_handler_t));
    g_display_handler.on_console_message = on_console_message;

    init_base(&g_load_handler.base, sizeof(cef_load_handler_t));
    g_load_handler.on_loading_state_change = on_loading_state_change;
    g_load_handler.on_load_start = on_load_start;
    g_load_handler.on_load_end = on_load_end;
    g_load_handler.on_load_error = on_load_error;

    init_base(&g_render_handler.base, sizeof(cef_render_handler_t));
    g_render_handler.get_view_rect = get_view_rect;
    g_render_handler.on_paint = on_paint;
    g_render_handler.get_screen_info = get_screen_info;

    init_base(&g_life_span_handler.base, sizeof(cef_life_span_handler_t));
    g_life_span_handler.on_after_created = on_after_created;
    g_life_span_handler.do_close = do_close;
    g_life_span_handler.on_before_close = on_before_close;

    init_base(&g_client.base, sizeof(cef_client_t));
    g_client.get_life_span_handler = get_life_span_handler;
    g_client.get_load_handler = get_load_handler;
    g_client.get_render_handler = get_render_handler;
    g_client.get_display_handler = get_display_handler;

    // CEF settings
    cef_settings_t settings = {};
    settings.size = sizeof(cef_settings_t);
    settings.windowless_rendering_enabled = 1;
    settings.no_sandbox = 1;
    settings.log_severity = LOGSEVERITY_ERROR;
    settings.persist_session_cookies = 1;

    // Use wick-helper as the subprocess — it injects stealth patches at
    // V8 context creation (before page JS runs). This is the correct
    // injection point; on_load_start is too late for inline scripts.
    char exe_dir_tmp[PATH_MAX];
    if (get_exe_dir(exe_dir_tmp, sizeof(exe_dir_tmp)) == 0) {
        char helper_path[PATH_MAX];
        snprintf(helper_path, sizeof(helper_path), "%s/wick-helper", exe_dir_tmp);
        // Fall back to self if helper not found
        if (access(helper_path, X_OK) == 0) {
            cef_string_utf8_to_utf16(helper_path, strlen(helper_path),
                                      &settings.browser_subprocess_path);
        } else {
            char exe_path[PATH_MAX];
            ssize_t exe_len = readlink("/proc/self/exe", exe_path, sizeof(exe_path) - 1);
            if (exe_len > 0) {
                exe_path[exe_len] = '\0';
                cef_string_utf8_to_utf16(exe_path, strlen(exe_path),
                                          &settings.browser_subprocess_path);
            }
        }
    }

    char exe_dir[PATH_MAX];
    if (get_exe_dir(exe_dir, sizeof(exe_dir)) == 0) {
        char resources_path[PATH_MAX];
        snprintf(resources_path, sizeof(resources_path), "%s", exe_dir);
        cef_string_utf8_to_utf16(resources_path, strlen(resources_path),
                                  &settings.resources_dir_path);

        char locales_path[PATH_MAX];
        snprintf(locales_path, sizeof(locales_path), "%s/locales", exe_dir);
        cef_string_utf8_to_utf16(locales_path, strlen(locales_path),
                                  &settings.locales_dir_path);
    }

    char cache_path[PATH_MAX];
    snprintf(cache_path, sizeof(cache_path), "%s/.wick/cef-cache",
             getenv("HOME") ? getenv("HOME") : "/tmp");
    cef_string_utf8_to_utf16(cache_path, strlen(cache_path),
                              &settings.root_cache_path);

    // Profile cache path — required for cookie persistence (AWS WAF tokens, etc.)
    char profile_path[PATH_MAX];
    snprintf(profile_path, sizeof(profile_path), "%s/.wick/cef-cache/Default",
             getenv("HOME") ? getenv("HOME") : "/tmp");
    cef_string_utf8_to_utf16(profile_path, strlen(profile_path),
                              &settings.cache_path);

    if (!cef_initialize(&main_args, &settings, NULL, NULL)) {
        fprintf(stderr, "wick-renderer: cef_initialize failed\n");
        free(new_argv);
        return 1;
    }

    cef_window_info_t window_info = {};
    window_info.size = sizeof(cef_window_info_t);
    window_info.windowless_rendering_enabled = 1;

    cef_browser_settings_t browser_settings = {};
    browser_settings.size = sizeof(cef_browser_settings_t);
    browser_settings.windowless_frame_rate = 1;

    const char* initial_url = one_shot ? url_arg : "about:blank";
    cef_string_t cef_url = {};
    cef_string_utf8_to_utf16(initial_url, strlen(initial_url), &cef_url);

    cef_browser_host_create_browser_sync(
        &window_info, &g_client, &cef_url, &browser_settings, NULL, NULL);
    cef_string_clear(&cef_url);

    // Smart content-readiness loop:
    // After initial load, inject JS that polls document.body.innerText.length.
    // Once text content is substantial (>500 chars) or stabilizes for 1s,
    // dump the rendered DOM. This handles SPAs that lazy-load via XHR.
    int post_load_ms = 0;
    const int tick_ms = 50;
    int poll_injected = 0;
    int last_nav = g_nav_count;
    int total_ms = 0;
    const int absolute_timeout = 45000; // 45s absolute max

    while (!g_source_done) {
        cef_do_message_loop_work();
        usleep(tick_ms * 1000);
        total_ms += tick_ms;

        // Detect page reload (AWS WAF challenge, DataDome redirect, etc.)
        // Reset poller so we wait for the NEW page content.
        if (g_nav_count != last_nav) {
            last_nav = g_nav_count;
            post_load_ms = 0;
            poll_injected = 0;
            g_click_x = -1;
            g_clicked = 0;
            fprintf(stderr, "wick-renderer: navigation %d detected (challenge reload?)\n",
                    g_nav_count);
        }

        // Absolute timeout — force dump whatever is in the DOM
        if (total_ms >= absolute_timeout) {
            fprintf(stderr, "wick-renderer: absolute timeout (%ds), dumping DOM\n",
                    absolute_timeout / 1000);
            if (g_browser && !g_source_done) {
                cef_frame_t* f = g_browser->get_main_frame(g_browser);
                if (f) {
                    char js[256];
                    snprintf(js, sizeof(js),
                        "console.log('%s'+document.documentElement.outerHTML);",
                        g_marker);
                    cef_string_t cef_js = {};
                    cef_string_utf8_to_utf16(js, strlen(js), &cef_js);
                    f->execute_java_script(f, &cef_js, NULL, 0);
                    cef_string_clear(&cef_js);
                }
                for (int i = 0; i < 100 && !g_source_done; i++) {
                    cef_do_message_loop_work();
                    usleep(10000);
                }
            }
            break;
        }

        if (g_load_done) {
            post_load_ms += tick_ms;

            // Inject the content-readiness poller once
            if (!poll_injected && g_browser) {
                cef_frame_t* frame = g_browser->get_main_frame(g_browser);
                if (frame) {
                    // JS poller: checks text length every 300ms.
                    // When content is >500 chars and stable for 1s, dumps DOM.
                    // Refuses to count the page as stable while a
                    // Cloudflare "Just a moment" interstitial is on
                    // screen — see renderer.m for full rationale.
                    char js[1400];
                    snprintf(js, sizeof(js),
                        "(function(){"
                        "var last=0,stable=0,t=0;"
                        "var iv=setInterval(function(){"
                        "  var body=document.body;"
                        "  var bt=body?body.innerText:'';"
                        "  var len=bt.length;"
                        "  var blt=bt.toLowerCase();"
                        "  var isCF=blt.indexOf('performing security verification')>=0||"
                        "           blt.indexOf('checking your browser')>=0;"
                        "  t+=300;"
                        "  if(isCF){stable=0;last=len;}"
                        "  else if(len>500&&len===last){stable+=300;}"
                        "  else{stable=0;last=len;}"
                        "  var stableHit=!isCF&&len>500&&stable>=1000;"
                        "  var cap=isCF?30000:8000;"
                        "  if(stableHit||t>=cap){"
                        "    clearInterval(iv);"
                        "    console.log('%s'+document.documentElement.outerHTML);"
                        "  }"
                        "},300);"
                        "})();", g_marker);
                    cef_string_t cef_js = {};
                    cef_string_utf8_to_utf16(js, strlen(js), &cef_js);
                    frame->execute_java_script(frame, &cef_js, NULL, 0);
                    cef_string_clear(&cef_js);
                }
                poll_injected = 1;
            }

            // Turnstile/CAPTCHA challenge detection and auto-click.
            // At 2s post-load, inject JS to find challenge iframes.
            // If found, report coordinates via console.log, then we click.
            if (post_load_ms >= 2000 && !g_clicked && g_click_x == -1 && g_browser) {
                cef_frame_t* frame = g_browser->get_main_frame(g_browser);
                if (frame) {
                    char js[512];
                    snprintf(js, sizeof(js),
                        "(function(){"
                        "var f=document.querySelector("
                        "'iframe[src*=\"challenges.cloudflare.com\"],"
                        "iframe[src*=\"captcha-delivery.com\"],"
                        "iframe[src*=\"recaptcha\"],"
                        "iframe[src*=\"hcaptcha\"]');"
                        "if(f){"
                        "  var r=f.getBoundingClientRect();"
                        "  if(r.width>0){"
                        "    console.log('%s'+"
                        "      Math.round(r.x+r.width/2)+','+"
                        "      Math.round(r.y+r.height/2));"
                        "  }"
                        "}"
                        "})();", g_click_prefix);
                    cef_string_t cef_js = {};
                    cef_string_utf8_to_utf16(js, strlen(js), &cef_js);
                    frame->execute_java_script(frame, &cef_js, NULL, 0);
                    cef_string_clear(&cef_js);
                }
            }

            // If we got click coordinates, send a mouse click event
            if (g_click_x >= 0 && !g_clicked && g_browser) {
                cef_browser_host_t* host = g_browser->get_host(g_browser);
                if (host) {
                    cef_mouse_event_t mouse = {};
                    mouse.x = g_click_x;
                    mouse.y = g_click_y;

                    // Move mouse to position first (more realistic)
                    host->send_mouse_move_event(host, &mouse, 0);
                    cef_do_message_loop_work();
                    usleep(100000); // 100ms pause

                    // Click
                    host->send_mouse_click_event(host, &mouse, MBT_LEFT, 0, 1);
                    cef_do_message_loop_work();
                    usleep(50000);
                    host->send_mouse_click_event(host, &mouse, MBT_LEFT, 1, 1);

                    fprintf(stderr, "wick-renderer: clicked challenge at %d,%d\n",
                            g_click_x, g_click_y);
                    g_clicked = 1;

                    // Reset timers — give page time to verify and redirect
                    post_load_ms = 0;
                    g_load_done = 0;
                    poll_injected = 0;
                }
            }

            // Safety timeout: 12s total after load (or after click)
            if (post_load_ms >= 12000 && !g_source_done) {
                fprintf(stderr, "wick-renderer: timeout waiting for content\n");
                // Last resort: dump whatever we have
                if (g_browser) {
                    cef_frame_t* frame = g_browser->get_main_frame(g_browser);
                    if (frame) {
                        char js[256];
                        snprintf(js, sizeof(js),
                            "console.log('%s'+document.documentElement.outerHTML);",
                            g_marker);
                        cef_string_t cef_js = {};
                        cef_string_utf8_to_utf16(js, strlen(js), &cef_js);
                        frame->execute_java_script(frame, &cef_js, NULL, 0);
                        cef_string_clear(&cef_js);
                    }
                }
                // Wait for the console callback
                for (int i = 0; i < 100 && !g_source_done; i++) {
                    cef_do_message_loop_work();
                    usleep(10000);
                }
                break;
            }
        }
    }

    if (one_shot) {
        // Output captured HTML to stdout (set by console handler)
        if (g_html && g_html_len > 0) {
            fwrite(g_html, 1, g_html_len, stdout);
        }
        fflush(stdout);
        cef_shutdown();
        free(new_argv);
        return 0;
    }

    // ── Daemon mode: read URLs from stdin, output length-prefixed HTML ──
    // Wait for browser to be ready
    while (!g_browser) {
        cef_do_message_loop_work();
        usleep(10000);
    }

    char line[8192];
    while (fgets(line, sizeof(line), stdin)) {
        size_t len = strlen(line);
        while (len > 0 && (line[len-1] == '\n' || line[len-1] == '\r'))
            line[--len] = '\0';
        if (len == 0) break;

        // Protocol: "URL" or "URL\tSELECTOR". Mirrors renderer.m.
        g_selector[0] = '\0';
        {
            char* tab = strchr(line, '\t');
            if (tab) {
                *tab = '\0';
                const char* sel = tab + 1;
                while (*sel == ' ' || *sel == '\t') sel++;
                char* dst = g_selector;
                size_t dst_cap = sizeof(g_selector) - 1;
                for (; *sel && (size_t)(dst - g_selector) < dst_cap; sel++) {
                    if (*sel == '\'' || *sel == '\\') continue;
                    *dst++ = *sel;
                }
                *dst = '\0';
            }
        }

        // Reset state for new request
        g_load_done = 0;
        g_source_done = 0;
        g_click_x = -1;
        g_click_y = -1;
        g_clicked = 0;
        g_nav_count = 0;
        free(g_html);
        g_html = NULL;
        g_html_len = 0;

        // Navigate to the URL
        cef_frame_t* frame = g_browser->get_main_frame(g_browser);
        if (!frame) {
            fprintf(stdout, "0\n");
            fflush(stdout);
            continue;
        }
        cef_string_t nav_url = {};
        cef_string_utf8_to_utf16(line, strlen(line), &nav_url);
        frame->load_url(frame, &nav_url);
        cef_string_clear(&nav_url);

        // Smart content polling (same as one-shot but outputs length-prefixed)
        int post_load_ms = 0;
        int poll_injected = 0;
        int last_nav = g_nav_count;
        int total_ms = 0;
        // Bump outer caps when a selector gate is in play — see renderer.m.
        const int post_load_cap = g_selector[0] ? 30000 : 12000;
        const int total_cap = g_selector[0] ? 60000 : 45000;

        while (!g_source_done) {
            cef_do_message_loop_work();
            usleep(50 * 1000);
            total_ms += 50;

            if (g_nav_count != last_nav) {
                last_nav = g_nav_count;
                post_load_ms = 0;
                poll_injected = 0;
            }

            if (total_ms >= total_cap) {
                if (g_browser && !g_source_done) {
                    cef_frame_t* f = g_browser->get_main_frame(g_browser);
                    if (f) {
                        char js[256];
                        snprintf(js, sizeof(js),
                            "console.log('%s'+document.documentElement.outerHTML);",
                            g_marker);
                        cef_string_t cef_js = {};
                        cef_string_utf8_to_utf16(js, strlen(js), &cef_js);
                        f->execute_java_script(f, &cef_js, NULL, 0);
                        cef_string_clear(&cef_js);
                    }
                    for (int i = 0; i < 100 && !g_source_done; i++) {
                        cef_do_message_loop_work();
                        usleep(10000);
                    }
                }
                break;
            }

            if (g_load_done) {
                post_load_ms += 50;

                if (!poll_injected && g_browser) {
                    cef_frame_t* f = g_browser->get_main_frame(g_browser);
                    if (f) {
                        // Daemon poller. Cloudflare-aware — refuses to
                        // dump while a "Just a moment" / "Performing
                        // security verification" interstitial is
                        // visible. See renderer.m for the macOS twin.
                        char js[2400];
                        snprintf(js, sizeof(js),
                            "(function(){"
                            "var sel=%s%s%s;"
                            "var last=0,stable=0,t=0;"
                            "var iv=setInterval(function(){"
                            "  var body=document.body;"
                            "  var bt=body?body.innerText:'';"
                            "  var len=bt.length;"
                            "  var blt=bt.toLowerCase();"
                            "  var isCF=blt.indexOf('performing security verification')>=0||"
                            "           blt.indexOf('checking your browser')>=0;"
                            "  t+=300;"
                            "  if(isCF){stable=0;last=len;}"
                            "  else if(len>500&&len===last){stable+=300;}"
                            "  else{stable=0;last=len;}"
                            "  var hit=sel?!!document.querySelector(sel):true;"
                            "  var stableHit=!isCF&&len>500&&stable>=1000&&hit;"
                            "  var cap=isCF?30000:(sel?20000:8000);"
                            "  if(stableHit||t>=cap){"
                            "    clearInterval(iv);"
                            "    console.log('%s'+document.documentElement.outerHTML);"
                            "  }"
                            "},300);"
                            "})();",
                            g_selector[0] ? "'" : "",
                            g_selector[0] ? g_selector : "null",
                            g_selector[0] ? "'" : "",
                            g_marker);
                        cef_string_t cef_js = {};
                        cef_string_utf8_to_utf16(js, strlen(js), &cef_js);
                        f->execute_java_script(f, &cef_js, NULL, 0);
                        cef_string_clear(&cef_js);
                    }
                    poll_injected = 1;
                }

                if (post_load_ms >= post_load_cap && !g_source_done) {
                    if (g_browser) {
                        cef_frame_t* f = g_browser->get_main_frame(g_browser);
                        if (f) {
                            char js[256];
                            snprintf(js, sizeof(js),
                                "console.log('%s'+document.documentElement.outerHTML);",
                                g_marker);
                            cef_string_t cef_js = {};
                            cef_string_utf8_to_utf16(js, strlen(js), &cef_js);
                            f->execute_java_script(f, &cef_js, NULL, 0);
                            cef_string_clear(&cef_js);
                        }
                    }
                    for (int i = 0; i < 100 && !g_source_done; i++) {
                        cef_do_message_loop_work();
                        usleep(10000);
                    }
                    break;
                }
            }
        }

        // Output length-prefixed response
        fprintf(stdout, "%zu\n", g_html_len);
        if (g_html && g_html_len > 0) {
            fwrite(g_html, 1, g_html_len, stdout);
        }
        fflush(stdout);
    }

    cef_shutdown();
    free(new_argv);
    return 0;
}
