package fetch

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"

	"github.com/myleshorton/wick/internal/engine"
	"github.com/myleshorton/wick/internal/extract"
)

// Request describes what to fetch and how.
type Request struct {
	URL           string
	Format        extract.Format
	RespectRobots bool
	Ctx           context.Context
}

// Result is the output of a successful fetch.
type Result struct {
	Content    string
	Title      string
	URL        string
	StatusCode int
	TimingMs   int64
}

// Fetcher orchestrates: URL validation → robots.txt → HTTP request → extraction.
type Fetcher struct {
	engine *engine.Engine
}

func NewFetcher(eng *engine.Engine) *Fetcher {
	return &Fetcher{engine: eng}
}

func (f *Fetcher) Fetch(req Request) (*Result, error) {
	start := time.Now()

	u, err := url.Parse(req.URL)
	if err != nil {
		return nil, fmt.Errorf("invalid URL: %w", err)
	}
	if u.Scheme != "http" && u.Scheme != "https" {
		return nil, fmt.Errorf("unsupported scheme %q (only http and https)", u.Scheme)
	}
	if u.Host == "" {
		return nil, fmt.Errorf("missing host in URL")
	}

	client, err := f.engine.Client()
	if err != nil {
		return nil, fmt.Errorf("engine: %w", err)
	}

	// robots.txt
	if req.RespectRobots {
		allowed, _ := CheckRobots(client, req.URL)
		if !allowed {
			return &Result{
				Content:    fmt.Sprintf("Blocked by robots.txt: %s disallows this path for automated agents.\nUse respect_robots=false to override (the user takes responsibility).", u.Host),
				URL:        req.URL,
				StatusCode: 0,
				TimingMs:   time.Since(start).Milliseconds(),
			}, nil
		}
	}

	ctx := req.Ctx
	if ctx == nil {
		var cancel context.CancelFunc
		ctx, cancel = context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
	}

	httpReq, err := http.NewRequestWithContext(ctx, "GET", req.URL, nil)
	if err != nil {
		return nil, fmt.Errorf("create request: %w", err)
	}

	// Apply Chrome-equivalent headers
	for key, vals := range engine.ChromeHeaders(req.URL) {
		for _, v := range vals {
			httpReq.Header.Add(key, v)
		}
	}

	resp, err := client.Do(httpReq)
	if err != nil {
		return nil, fmt.Errorf("fetch: %w", err)
	}
	defer resp.Body.Close()

	// Detect challenge / block pages
	if resp.StatusCode == 403 || resp.StatusCode == 503 {
		body, _ := io.ReadAll(resp.Body)
		if isChallengeResponse(string(body)) {
			return &Result{
				Content:    "This page returned a CAPTCHA or browser challenge. The content could not be extracted automatically.",
				URL:        req.URL,
				StatusCode: resp.StatusCode,
				TimingMs:   time.Since(start).Milliseconds(),
			}, nil
		}
		// Not a challenge — just an error
		return &Result{
			Content:    fmt.Sprintf("HTTP %d: %s\n\n%s", resp.StatusCode, resp.Status, string(body)),
			URL:        req.URL,
			StatusCode: resp.StatusCode,
			TimingMs:   time.Since(start).Milliseconds(),
		}, nil
	}

	if resp.StatusCode >= 400 {
		body, _ := io.ReadAll(resp.Body)
		return &Result{
			Content:    fmt.Sprintf("HTTP %d: %s\n\n%s", resp.StatusCode, resp.Status, string(body)),
			URL:        req.URL,
			StatusCode: resp.StatusCode,
			TimingMs:   time.Since(start).Milliseconds(),
		}, nil
	}

	format := req.Format
	if format == "" {
		format = extract.FormatMarkdown
	}

	extracted, err := extract.Extract(resp.Body, u, format)
	if err != nil {
		return nil, fmt.Errorf("extraction: %w", err)
	}

	return &Result{
		Content:    extracted.Content,
		Title:      extracted.Title,
		URL:        req.URL,
		StatusCode: resp.StatusCode,
		TimingMs:   time.Since(start).Milliseconds(),
	}, nil
}

func isChallengeResponse(body string) bool {
	signatures := []string{
		"challenges.cloudflare.com",
		"cf-browser-verification",
		"just a moment...",
		"checking your browser",
		"google.com/recaptcha",
		"hcaptcha.com",
	}
	lower := strings.ToLower(body)
	for _, sig := range signatures {
		if strings.Contains(lower, sig) {
			return true
		}
	}
	return false
}
