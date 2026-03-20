package engine

import "net/http"

// Chrome version must match the Cronet/Chromium version used by
// sagernet/cronet-go (currently 143). Mismatched UA + Client Hints
// vs TLS fingerprint is a detection vector.
const (
	chromeMajor   = "143"
	chromeFullVer = "143.0.7499.109"
)

// ChromeUserAgent returns a current Chrome User-Agent string.
func ChromeUserAgent() string {
	return "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/" + chromeFullVer + " Safari/537.36"
}

// ChromeHeaders builds the full set of headers that Chrome sends on a
// top-level navigation request. These complement the TLS/HTTP2 fingerprint
// that Cronet provides at the network layer.
func ChromeHeaders(targetURL string) http.Header {
	h := http.Header{}
	h.Set("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8")
	h.Set("Accept-Language", "en-US,en;q=0.9")
	h.Set("Accept-Encoding", "gzip, deflate, br, zstd")
	h.Set("Cache-Control", "max-age=0")
	h.Set("Sec-Ch-Ua", `"Chromium";v="`+chromeMajor+`", "Google Chrome";v="`+chromeMajor+`", "Not:A-Brand";v="24"`)
	h.Set("Sec-Ch-Ua-Mobile", "?0")
	h.Set("Sec-Ch-Ua-Platform", `"macOS"`)
	h.Set("Sec-Fetch-Dest", "document")
	h.Set("Sec-Fetch-Mode", "navigate")
	h.Set("Sec-Fetch-Site", "none")
	h.Set("Sec-Fetch-User", "?1")
	h.Set("Upgrade-Insecure-Requests", "1")
	return h
}
