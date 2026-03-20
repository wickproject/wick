package fetch

import (
	"io"
	"net/http"
	"net/url"
	"sync"
	"time"

	"github.com/temoto/robotstxt"
)

const (
	robotsTTL     = 1 * time.Hour
	maxCacheHosts = 500
)

type rCache struct {
	mu      sync.RWMutex
	entries map[string]*rEntry
}

type rEntry struct {
	data      *robotstxt.RobotsData
	fetchedAt time.Time
}

var robotsCache = &rCache{entries: make(map[string]*rEntry)}

// CheckRobots returns true if the URL is allowed by robots.txt.
// Checks both the "Wick" and "*" user agents.
// Returns true (allowed) if robots.txt can't be fetched.
func CheckRobots(client *http.Client, targetURL string) (bool, error) {
	u, err := url.Parse(targetURL)
	if err != nil {
		return false, err
	}

	host := u.Scheme + "://" + u.Host
	data, err := getRobotsData(client, host)
	if err != nil {
		return true, nil // can't fetch robots.txt → allow
	}

	// Check the "Wick" agent first, then fall back to "*"
	if group := data.FindGroup("Wick"); !group.Test(u.Path) {
		return false, nil
	}
	if group := data.FindGroup("*"); !group.Test(u.Path) {
		return false, nil
	}
	return true, nil
}

func getRobotsData(client *http.Client, host string) (*robotstxt.RobotsData, error) {
	robotsCache.mu.RLock()
	entry, ok := robotsCache.entries[host]
	robotsCache.mu.RUnlock()

	if ok && time.Since(entry.fetchedAt) < robotsTTL {
		return entry.data, nil
	}

	resp, err := client.Get(host + "/robots.txt")
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	// Only cache allow-all for definitive "no robots.txt" responses.
	// Transient errors (500, 503, 429) should not be cached as allow-all.
	switch {
	case resp.StatusCode == http.StatusOK:
		// Parse and cache the actual robots.txt
		body, err := io.ReadAll(resp.Body)
		if err != nil {
			return nil, err
		}
		data, err := robotstxt.FromBytes(body)
		if err != nil {
			return nil, err
		}
		cacheRobots(host, data)
		return data, nil

	case resp.StatusCode == http.StatusNotFound || resp.StatusCode == http.StatusGone:
		// 404/410: no robots.txt exists → cache as allow-all
		data, _ := robotstxt.FromBytes([]byte(""))
		cacheRobots(host, data)
		return data, nil

	default:
		// Transient error (500, 503, 429, 401, etc.) — don't cache,
		// return allow-all for this request only.
		data, _ := robotstxt.FromBytes([]byte(""))
		return data, nil
	}
}

func cacheRobots(host string, data *robotstxt.RobotsData) {
	robotsCache.mu.Lock()
	defer robotsCache.mu.Unlock()

	// Evict expired entries when the cache gets large
	if len(robotsCache.entries) >= maxCacheHosts {
		now := time.Now()
		for k, e := range robotsCache.entries {
			if now.Sub(e.fetchedAt) >= robotsTTL {
				delete(robotsCache.entries, k)
			}
		}
	}

	robotsCache.entries[host] = &rEntry{data: data, fetchedAt: time.Now()}
}
