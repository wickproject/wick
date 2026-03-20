//go:build darwin && arm64 && !with_purego

// Package darwin_arm64 links the prebuilt Cronet static library for macOS arm64.
// Blank-import this package to statically link libcronet into the binary.
package darwin_arm64

// #cgo LDFLAGS: ${SRCDIR}/libcronet.a -lbsm -lpmenergy -lpmsample -lresolv -framework CoreFoundation -framework CoreGraphics -framework CoreText -framework Foundation -framework Security -framework ApplicationServices -framework AppKit -framework IOKit -framework OpenDirectory -framework CFNetwork -framework CoreServices -framework Network -framework SystemConfiguration -framework UniformTypeIdentifiers -framework CryptoTokenKit -framework LocalAuthentication
import "C"
