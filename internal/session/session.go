package session

import (
	"fmt"
	"os"
	"path/filepath"
)

// StoragePath returns the default Wick data directory.
func StoragePath() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("cannot determine home directory: %w", err)
	}
	return filepath.Join(home, ".wick", "data"), nil
}

// ClearSession removes all persistent data (cookies, cache) and
// recreates the directory.
func ClearSession() error {
	path, err := StoragePath()
	if err != nil {
		return err
	}
	if err := os.RemoveAll(path); err != nil {
		return err
	}
	return os.MkdirAll(path, 0700)
}
