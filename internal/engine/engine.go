package engine

import (
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"sync"

	cronet "github.com/sagernet/cronet-go"
)

// Config controls the Cronet engine behaviour.
type Config struct {
	StoragePath string // persistent cookies + cache
	UserAgent   string
	EnableQUIC  bool
	CacheSize   int64 // bytes
}

// DefaultConfig returns sensible defaults for local use.
func DefaultConfig() (Config, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return Config{}, fmt.Errorf("cannot determine home directory: %w", err)
	}
	return Config{
		StoragePath: filepath.Join(home, ".wick", "data"),
		UserAgent:   ChromeUserAgent(),
		EnableQUIC:  true,
		CacheSize:   50 * 1024 * 1024, // 50 MB
	}, nil
}

// Engine wraps a Cronet engine and exposes an *http.Client whose TLS,
// HTTP/2, and QUIC behaviour is identical to Chrome.
// Initialization is lazy — the native engine starts on first Client() call.
type Engine struct {
	once     sync.Once
	initErr  error
	mu       sync.Mutex
	closed   bool
	engine   cronet.Engine
	executor cronet.Executor
	client   *http.Client
	cfg      Config
}

// New creates an Engine with the given config. The Cronet native engine
// is NOT started until the first call to Client().
func New(cfg Config) *Engine {
	return &Engine{cfg: cfg}
}

func (e *Engine) init() error {
	e.once.Do(func() {
		e.initErr = e.start()
	})
	return e.initErr
}

func (e *Engine) start() error {
	if err := loadCronetLibrary(); err != nil {
		return err
	}

	if err := os.MkdirAll(e.cfg.StoragePath, 0700); err != nil {
		return fmt.Errorf("create storage dir: %w", err)
	}

	params := cronet.NewEngineParams()
	defer params.Destroy()

	params.SetEnableHTTP2(true)
	params.SetEnableQuic(e.cfg.EnableQUIC)
	params.SetEnableBrotli(true)
	params.SetUserAgent(e.cfg.UserAgent)
	params.SetStoragePath(e.cfg.StoragePath)
	params.SetHTTPCacheMode(cronet.HTTPCacheModeDisk)
	params.SetHTTPCacheMaxSize(e.cfg.CacheSize)
	params.SetEnableCheckResult(false) // return error codes, don't SIGABRT

	eng := cronet.NewEngine()
	result := eng.StartWithParams(params)
	if result != cronet.ResultSuccess {
		eng.Destroy()
		return fmt.Errorf("cronet engine start failed: result=%d", result)
	}

	executor := cronet.NewExecutor(func(executor cronet.Executor, command cronet.Runnable) {
		go func() {
			command.Run()
			command.Destroy()
		}()
	})

	transport := &cronet.RoundTripper{
		Engine:   eng,
		Executor: executor,
	}

	e.engine = eng
	e.executor = executor
	e.client = &http.Client{Transport: transport}
	return nil
}

// Client returns the Cronet-backed HTTP client.
// The native engine is started on the first call.
func (e *Engine) Client() (*http.Client, error) {
	if err := e.init(); err != nil {
		return nil, err
	}
	return e.client, nil
}

// StoragePath returns the path used for persistent data.
func (e *Engine) StoragePath() string {
	return e.cfg.StoragePath
}

// Close shuts down the Cronet engine and frees native resources.
// Safe to call even if the engine was never started. Idempotent.
func (e *Engine) Close() error {
	e.mu.Lock()
	defer e.mu.Unlock()
	if e.closed {
		return nil
	}
	e.closed = true
	if e.engine != (cronet.Engine{}) {
		e.engine.Shutdown()
		e.engine.Destroy()
		e.executor.Destroy()
		e.engine = cronet.Engine{}
		e.executor = cronet.Executor{}
		e.client = nil
	}
	return nil
}
