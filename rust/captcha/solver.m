// wick-captcha: Opens a WKWebView for the user to solve a CAPTCHA.
//
// Usage: wick-captcha <url>
// Opens a small window with the URL. When the user solves the CAPTCHA,
// outputs the domain cookies as JSON to stdout and exits.
//
// The User-Agent is set to match Wick/Cronet's Chrome UA so that the
// clearance cookie is valid for subsequent Wick requests.

#import <Cocoa/Cocoa.h>
#import <WebKit/WebKit.h>
#import <UserNotifications/UserNotifications.h>

static const char* CHROME_UA =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) "
    "AppleWebKit/537.36 (KHTML, like Gecko) "
    "Chrome/143.0.7499.109 Safari/537.36";

// ── Delegate: monitors navigation and cookies ─────────────────

@interface CaptchaSolverDelegate : NSObject <WKNavigationDelegate, NSApplicationDelegate>
@property (nonatomic, strong) WKWebView* webView;
@property (nonatomic, strong) NSWindow* window;
@property (nonatomic, copy) NSString* originalHost;
@property (nonatomic, assign) BOOL solved;
@property (nonatomic, assign) int challengeCount;
@end

@implementation CaptchaSolverDelegate

- (void)webView:(WKWebView*)webView
    decidePolicyForNavigationResponse:(WKNavigationResponse*)navigationResponse
    decisionHandler:(void (^)(WKNavigationResponsePolicy))decisionHandler {

    // Check if this is an HTTP response (not about:blank, etc.)
    if ([navigationResponse.response isKindOfClass:[NSHTTPURLResponse class]]) {
        NSHTTPURLResponse* httpResponse = (NSHTTPURLResponse*)navigationResponse.response;
        NSInteger statusCode = httpResponse.statusCode;

        // If we get a 200 after being on a challenge page, the CAPTCHA was solved
        if (statusCode == 200 && self.challengeCount > 0 && !self.solved) {
            self.solved = YES;
            // Small delay to let cookies settle
            dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(1.0 * NSEC_PER_SEC)),
                dispatch_get_main_queue(), ^{
                    [self extractCookiesAndExit];
                });
        }

        // Track challenge responses
        if (statusCode == 403 || statusCode == 503) {
            self.challengeCount++;
        }
    }

    decisionHandler(WKNavigationResponsePolicyAllow);
}

- (void)webView:(WKWebView*)webView
    didFinishNavigation:(WKNavigation*)navigation {

    // Also check URL for successful navigation past challenge
    NSString* currentURL = webView.URL.absoluteString;

    // Cloudflare challenges redirect to the original URL after solving
    if (self.challengeCount > 0 && !self.solved) {
        // Check for clearance cookies
        [self checkForClearanceCookies];
    }
}

- (void)checkForClearanceCookies {
    WKHTTPCookieStore* cookieStore = self.webView.configuration.websiteDataStore.httpCookieStore;
    [cookieStore getAllCookies:^(NSArray<NSHTTPCookie*>* cookies) {
        for (NSHTTPCookie* cookie in cookies) {
            // cf_clearance is the key Cloudflare clearance cookie
            if ([cookie.name isEqualToString:@"cf_clearance"] ||
                [cookie.name isEqualToString:@"__cf_bm"]) {
                if (!self.solved) {
                    self.solved = YES;
                    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(0.5 * NSEC_PER_SEC)),
                        dispatch_get_main_queue(), ^{
                            [self extractCookiesAndExit];
                        });
                }
                return;
            }
        }
    }];
}

- (void)extractCookiesAndExit {
    WKHTTPCookieStore* cookieStore = self.webView.configuration.websiteDataStore.httpCookieStore;
    [cookieStore getAllCookies:^(NSArray<NSHTTPCookie*>* cookies) {
        NSMutableArray* cookieList = [NSMutableArray array];

        for (NSHTTPCookie* cookie in cookies) {
            // Only output cookies for the original domain
            if ([cookie.domain hasSuffix:self.originalHost] ||
                [self.originalHost hasSuffix:cookie.domain]) {
                [cookieList addObject:@{
                    @"name": cookie.name,
                    @"value": cookie.value,
                    @"domain": cookie.domain,
                    @"path": cookie.path ?: @"/",
                    @"secure": @(cookie.isSecure),
                    @"httpOnly": @(cookie.isHTTPOnly),
                }];
            }
        }

        NSError* error;
        NSData* jsonData = [NSJSONSerialization dataWithJSONObject:cookieList
                                                           options:0
                                                             error:&error];
        if (jsonData) {
            fwrite(jsonData.bytes, 1, jsonData.length, stdout);
            fprintf(stdout, "\n");
            fflush(stdout);
        }

        // Exit
        dispatch_async(dispatch_get_main_queue(), ^{
            [NSApp terminate:nil];
        });
    }];
}

- (void)applicationDidFinishLaunching:(NSNotification*)notification {
    // Bring window to front
    [NSApp activateIgnoringOtherApps:YES];
    [self.window makeKeyAndOrderFront:nil];
}

- (BOOL)applicationShouldTerminateAfterLastWindowClosed:(NSApplication*)sender {
    return YES;
}

@end

// ── Notification helper ───────────────────────────────────────

static void sendNotification(const char* host) {
    // Use NSUserNotification for simple macOS notification
    NSUserNotification* notification = [[NSUserNotification alloc] init];
    notification.title = @"Wick needs your help";
    notification.informativeText = [NSString stringWithFormat:
        @"Solve a CAPTCHA for %s to continue", host];
    notification.soundName = NSUserNotificationDefaultSoundName;
    [[NSUserNotificationCenter defaultUserNotificationCenter]
        deliverNotification:notification];
}

// ── Main ──────────────────────────────────────────────────────

int main(int argc, char* argv[]) {
    if (argc < 2) {
        fprintf(stderr, "Usage: wick-captcha <url>\n");
        return 1;
    }

    @autoreleasepool {
        NSString* urlString = [NSString stringWithUTF8String:argv[1]];
        NSURL* url = [NSURL URLWithString:urlString];

        if (!url || !url.host) {
            fprintf(stderr, "wick-captcha: invalid URL\n");
            return 1;
        }

        [NSApplication sharedApplication];
        [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];

        // Create WKWebView with Chrome User-Agent
        WKWebViewConfiguration* config = [[WKWebViewConfiguration alloc] init];
        config.applicationNameForUserAgent = @"";

        WKWebView* webView = [[WKWebView alloc] initWithFrame:NSMakeRect(0, 0, 500, 600)
                                                 configuration:config];
        webView.customUserAgent = [NSString stringWithUTF8String:CHROME_UA];

        // Create window
        NSWindow* window = [[NSWindow alloc]
            initWithContentRect:NSMakeRect(0, 0, 500, 600)
                      styleMask:(NSWindowStyleMaskTitled |
                                 NSWindowStyleMaskClosable |
                                 NSWindowStyleMaskResizable)
                        backing:NSBackingStoreBuffered
                          defer:NO];
        window.title = [NSString stringWithFormat:@"Wick — Solve CAPTCHA for %@", url.host];
        window.contentView = webView;
        [window center];

        // Set up delegate
        CaptchaSolverDelegate* delegate = [[CaptchaSolverDelegate alloc] init];
        delegate.webView = webView;
        delegate.window = window;
        delegate.originalHost = url.host;
        delegate.solved = NO;
        delegate.challengeCount = 0;
        webView.navigationDelegate = delegate;
        NSApp.delegate = delegate;

        // Send notification
        sendNotification(url.host.UTF8String);

        // Load the CAPTCHA URL
        [webView loadRequest:[NSURLRequest requestWithURL:url]];

        // Run the app
        [NSApp run];
    }

    return 0;
}
