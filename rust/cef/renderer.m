// wick-renderer: Persistent CEF offscreen renderer daemon.
//
// Protocol (stdin/stdout):
//   Request:  URL as a line terminated by \n
//   Response: decimal byte count as a line, then exactly that many bytes of HTML
//
// Stays alive between requests — CEF and the browser instance are reused.
// Send EOF (close stdin) or empty line to shut down.

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <libgen.h>
#include <mach-o/dyld.h>
#include <unistd.h>
#include <fcntl.h>

#import <Cocoa/Cocoa.h>

#include "include/capi/cef_app_capi.h"
#include "include/capi/cef_browser_capi.h"
#include "include/capi/cef_client_capi.h"
#include "include/capi/cef_frame_capi.h"
#include "include/capi/cef_life_span_handler_capi.h"
#include "include/capi/cef_load_handler_capi.h"
#include "include/capi/cef_render_handler_capi.h"
#include "include/capi/cef_string_visitor_capi.h"
#include "include/cef_api_hash.h"

// ── State ─────────────────────────────────────────────────────

static cef_browser_t* g_browser = NULL;
static int g_page_loaded = 0;      // set when isLoading transitions to 0
static int g_source_ready = 0;     // set when visitor_visit fires
static char* g_html = NULL;        // rendered HTML (malloc'd)
static size_t g_html_len = 0;

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

// ── String visitor — captures rendered HTML ───────────────────

static void CEF_CALLBACK visitor_visit(cef_string_visitor_t* self,
                                        const cef_string_t* string) {
    (void)self;
    free(g_html);
    g_html = NULL;
    g_html_len = 0;

    if (string && string->str) {
        cef_string_utf8_t utf8 = {};
        cef_string_utf16_to_utf8(string->str, string->length, &utf8);
        g_html = malloc(utf8.length);
        if (g_html) {
            memcpy(g_html, utf8.str, utf8.length);
            g_html_len = utf8.length;
        }
        cef_string_utf8_clear(&utf8);
    }
    g_source_ready = 1;
}

static cef_string_visitor_t g_html_visitor;

// ── Load handler ──────────────────────────────────────────────

static void CEF_CALLBACK on_loading_state_change(
    cef_load_handler_t* self, cef_browser_t* browser,
    int isLoading, int canGoBack, int canGoForward) {
    (void)self; (void)canGoBack; (void)canGoForward;
    if (!isLoading && browser) {
        g_page_loaded = 1;
    }
}

static void CEF_CALLBACK on_load_start(cef_load_handler_t* self,
    cef_browser_t* browser, cef_frame_t* frame, cef_transition_type_t tt) {
    (void)self; (void)browser; (void)frame; (void)tt;
}

static void CEF_CALLBACK on_load_end(cef_load_handler_t* self,
    cef_browser_t* browser, cef_frame_t* frame, int httpStatusCode) {
    (void)self; (void)browser; (void)frame; (void)httpStatusCode;
}

static void CEF_CALLBACK on_load_error(cef_load_handler_t* self,
    cef_browser_t* browser, cef_frame_t* frame,
    cef_errorcode_t errorCode, const cef_string_t* errorText,
    const cef_string_t* failedUrl) {
    (void)self; (void)browser; (void)errorText; (void)failedUrl;
    if (frame && frame->is_main(frame)) {
        fprintf(stderr, "wick-renderer: main frame load error %d\n", errorCode);
        g_page_loaded = 1; // unblock the wait loop even on error
    }
}

static cef_load_handler_t g_load_handler;

// ── Render handler (minimal OSR) ──────────────────────────────

static void CEF_CALLBACK get_view_rect(cef_render_handler_t* self,
    cef_browser_t* browser, cef_rect_t* rect) {
    (void)self; (void)browser;
    rect->x = 0; rect->y = 0; rect->width = 1; rect->height = 1;
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

static cef_client_t g_client;

// ── Render a single URL (called from the request loop) ────────

static void render_url(const char* url) {
    g_page_loaded = 0;
    g_source_ready = 0;
    free(g_html);
    g_html = NULL;
    g_html_len = 0;

    // Navigate the existing browser to the new URL
    cef_frame_t* frame = g_browser->get_main_frame(g_browser);
    if (!frame) {
        fprintf(stdout, "0\n");
        fflush(stdout);
        return;
    }

    cef_string_t cef_url = {};
    cef_string_utf8_to_utf16(url, strlen(url), &cef_url);
    frame->load_url(frame, &cef_url);
    cef_string_clear(&cef_url);

    // Pump message loop until page loads (with 30s timeout)
    int ticks = 0;
    while (!g_page_loaded && ticks < 3000) {
        cef_do_message_loop_work();
        usleep(10000); // 10ms
        ticks++;
    }

    if (!g_page_loaded) {
        // Timed out waiting for page load
        fprintf(stdout, "0\n");
        fflush(stdout);
        return;
    }

    // Extract source HTML
    frame = g_browser->get_main_frame(g_browser);
    if (frame) {
        frame->get_source(frame, &g_html_visitor);
    }

    // Pump until visitor fires (with 5s timeout)
    ticks = 0;
    while (!g_source_ready && ticks < 500) {
        cef_do_message_loop_work();
        usleep(10000);
        ticks++;
    }

    if (!g_source_ready) {
        fprintf(stdout, "0\n");
        fflush(stdout);
        return;
    }

    // Write length-prefixed response
    fprintf(stdout, "%zu\n", g_html_len);
    if (g_html && g_html_len > 0) {
        fwrite(g_html, 1, g_html_len, stdout);
    }
    fflush(stdout);
}

// ── Main ──────────────────────────────────────────────────────

int main(int argc, char* argv[]) {
    char exe_buf[4096];
    uint32_t exe_buf_size = sizeof(exe_buf);
    if (_NSGetExecutablePath(exe_buf, &exe_buf_size) != 0) {
        fprintf(stderr, "wick-renderer: executable path too long\n");
        return 1;
    }

    int new_argc = argc + 2;
    char** new_argv = malloc(sizeof(char*) * (new_argc + 1));
    for (int i = 0; i < argc; i++) new_argv[i] = argv[i];
    new_argv[argc] = "--disable-gpu";
    new_argv[argc + 1] = "--disable-gpu-compositing";
    new_argv[new_argc] = NULL;

    cef_api_hash(CEF_API_VERSION, 0);

    cef_main_args_t main_args = { .argc = new_argc, .argv = new_argv };
    int exit_code = cef_execute_process(&main_args, NULL, NULL);
    if (exit_code >= 0) {
        free(new_argv);
        return exit_code;
    }

    // Support one-shot mode: wick-renderer <url> (backward compat)
    int one_shot = (argc >= 2 && strncmp(argv[1], "--", 2) != 0);

    @autoreleasepool {
        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
    }

    // Initialize handlers
    init_base(&g_html_visitor.base, sizeof(cef_string_visitor_t));
    g_html_visitor.visit = visitor_visit;

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

    // CEF settings
    cef_settings_t settings = {};
    settings.size = sizeof(cef_settings_t);
    settings.windowless_rendering_enabled = 1;
    settings.no_sandbox = 1;
    settings.log_severity = LOGSEVERITY_ERROR;

    char* exe_dir = dirname(exe_buf);
    char fw_dir[4096];
    snprintf(fw_dir, sizeof(fw_dir), "%s/../Frameworks/Chromium Embedded Framework.framework", exe_dir);
    cef_string_utf8_to_utf16(fw_dir, strlen(fw_dir), &settings.framework_dir_path);

    char helper_path[4096];
    snprintf(helper_path, sizeof(helper_path),
             "%s/../Frameworks/wick Helper.app/Contents/MacOS/wick Helper", exe_dir);
    char helper_real[4096];
    if (realpath(helper_path, helper_real)) {
        cef_string_utf8_to_utf16(helper_real, strlen(helper_real), &settings.browser_subprocess_path);
    } else {
        fprintf(stderr, "wick-renderer: helper not found at %s\n", helper_path);
        return 1;
    }

    char bundle_path[4096];
    snprintf(bundle_path, sizeof(bundle_path), "%s/../..", exe_dir);
    char bundle_real[4096];
    if (realpath(bundle_path, bundle_real)) {
        cef_string_utf8_to_utf16(bundle_real, strlen(bundle_real), &settings.main_bundle_path);
    }

    char cache_path[4096];
    snprintf(cache_path, sizeof(cache_path), "%s/.wick/cef-cache",
             getenv("HOME") ? getenv("HOME") : "/tmp");
    cef_string_utf8_to_utf16(cache_path, strlen(cache_path), &settings.root_cache_path);

    if (!cef_initialize(&main_args, &settings, NULL, NULL)) {
        fprintf(stderr, "wick-renderer: cef_initialize failed\n");
        return 1;
    }

    // Create browser navigating to about:blank (or first URL in one-shot mode)
    cef_window_info_t window_info = {};
    window_info.size = sizeof(cef_window_info_t);
    window_info.windowless_rendering_enabled = 1;
    window_info.hidden = 1;

    cef_browser_settings_t browser_settings = {};
    browser_settings.size = sizeof(cef_browser_settings_t);
    browser_settings.windowless_frame_rate = 1;

    const char* initial_url = one_shot ? argv[1] : "about:blank";
    cef_string_t cef_url = {};
    cef_string_utf8_to_utf16(initial_url, strlen(initial_url), &cef_url);

    cef_browser_host_create_browser_sync(
        &window_info, &g_client, &cef_url, &browser_settings, NULL, NULL);
    cef_string_clear(&cef_url);

    if (one_shot) {
        // Legacy one-shot mode: render argv[1], output to stdout, exit
        cef_run_message_loop();
        cef_shutdown();
        free(new_argv);
        return 0;
    }

    // ── Persistent daemon mode ────────────────────────────────
    // Wait for initial browser creation
    while (!g_browser) {
        cef_do_message_loop_work();
        usleep(10000);
    }

    // Read URLs from stdin, render each, write results to stdout
    char line[8192];
    while (fgets(line, sizeof(line), stdin)) {
        // Strip trailing newline
        size_t len = strlen(line);
        while (len > 0 && (line[len-1] == '\n' || line[len-1] == '\r'))
            line[--len] = '\0';

        if (len == 0) break; // empty line = shutdown

        render_url(line);
    }

    cef_shutdown();
    free(g_html);
    free(new_argv);
    return 0;
}
