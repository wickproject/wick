VERSION ?= $(shell git describe --tags --always --dirty 2>/dev/null || echo "dev")
COMMIT  ?= $(shell git rev-parse --short HEAD 2>/dev/null || echo "unknown")
DATE    ?= $(shell date -u +"%Y-%m-%dT%H:%M:%SZ")
LDFLAGS := -X github.com/myleshorton/wick/pkg/version.Version=$(VERSION) \
           -X github.com/myleshorton/wick/pkg/version.Commit=$(COMMIT) \
           -X github.com/myleshorton/wick/pkg/version.Date=$(DATE)

.PHONY: build build-purego clean test download-lib

# Default: static CGO build on macOS arm64, purego everywhere else.
# macOS amd64 is not yet supported for static builds.
ifeq ($(shell uname -s)-$(shell uname -m),Darwin-arm64)
build:
	CGO_ENABLED=1 go build -ldflags "$(LDFLAGS)" -o wick ./cmd/wick
else
build:
	go build -tags with_purego -ldflags "$(LDFLAGS)" -o wick ./cmd/wick
endif

# Force purego build (requires libcronet.so/.dll/.dylib at runtime)
build-purego:
	go build -tags with_purego -ldflags "$(LDFLAGS)" -o wick ./cmd/wick

clean:
	rm -f wick

test:
	go test -tags with_purego ./...

download-lib:
	bash scripts/download-libcronet.sh .
