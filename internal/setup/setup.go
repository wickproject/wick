package setup

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
)

// Setup auto-configures detected MCP clients to use Wick.
func Setup() error {
	wickPath, err := os.Executable()
	if err != nil {
		return fmt.Errorf("find wick binary: %w", err)
	}

	// Prefer the Claude CLI if available — it manages its own config cleanly.
	if err := setupClaudeCLI(wickPath); err == nil {
		fmt.Println("Configured Wick for Claude Code (via claude mcp add)")
		return nil
	}

	configured := false

	if err := setupClaudeJSON(wickPath); err == nil {
		fmt.Println("Configured Wick for Claude Code (via ~/.claude.json)")
		configured = true
	}

	if err := setupCursor(wickPath); err == nil {
		fmt.Println("Configured Wick for Cursor")
		configured = true
	}

	if !configured {
		return fmt.Errorf("no MCP clients found — install Claude Code or Cursor, then run 'wick setup' again")
	}
	return nil
}

func setupClaudeCLI(wickPath string) error {
	cmd := exec.Command("claude", "mcp", "add",
		"--transport", "stdio",
		"--scope", "user",
		"wick", "--",
		wickPath, "serve", "--mcp",
	)
	return cmd.Run()
}

func setupClaudeJSON(wickPath string) error {
	home, err := os.UserHomeDir()
	if err != nil {
		return fmt.Errorf("cannot determine home directory: %w", err)
	}
	configPath := filepath.Join(home, ".claude.json")

	config := make(map[string]any)
	if data, err := os.ReadFile(configPath); err == nil {
		_ = json.Unmarshal(data, &config)
	}

	servers, ok := config["mcpServers"].(map[string]any)
	if !ok {
		servers = make(map[string]any)
	}

	servers["wick"] = map[string]any{
		"command": wickPath,
		"args":    []string{"serve", "--mcp"},
	}
	config["mcpServers"] = servers

	data, err := json.MarshalIndent(config, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(configPath, data, 0644)
}

func setupCursor(wickPath string) error {
	home, err := os.UserHomeDir()
	if err != nil {
		return fmt.Errorf("cannot determine home directory: %w", err)
	}
	configDir := filepath.Join(home, ".cursor")

	if _, err := os.Stat(configDir); os.IsNotExist(err) {
		return fmt.Errorf("cursor config directory not found")
	}

	configPath := filepath.Join(configDir, "mcp.json")

	config := make(map[string]any)
	if data, err := os.ReadFile(configPath); err == nil {
		_ = json.Unmarshal(data, &config)
	}

	servers, ok := config["mcpServers"].(map[string]any)
	if !ok {
		servers = make(map[string]any)
	}

	servers["wick"] = map[string]any{
		"command": wickPath,
		"args":    []string{"serve", "--mcp"},
	}
	config["mcpServers"] = servers

	data, err := json.MarshalIndent(config, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(configPath, data, 0644)
}
