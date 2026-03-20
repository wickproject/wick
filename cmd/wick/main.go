package main

import (
	"context"
	"fmt"
	"log/slog"
	"os"
	"os/signal"

	"github.com/spf13/cobra"

	"github.com/myleshorton/wick/internal/engine"
	"github.com/myleshorton/wick/internal/extract"
	"github.com/myleshorton/wick/internal/fetch"
	wickmcp "github.com/myleshorton/wick/internal/mcp"
	"github.com/myleshorton/wick/internal/setup"
	"github.com/myleshorton/wick/pkg/version"
)

func main() {
	root := &cobra.Command{
		Use:          "wick",
		Short:        "Browser-grade web access for AI agents",
		Long:         "Wick uses Chrome's actual network stack (Cronet) to give AI agents the same web access their human operators have.",
		SilenceUsage: true,
	}

	root.AddCommand(serveCmd())
	root.AddCommand(fetchCmd())
	root.AddCommand(setupCmd())
	root.AddCommand(versionCmd())

	if err := root.Execute(); err != nil {
		os.Exit(1)
	}
}

func serveCmd() *cobra.Command {
	var mcpMode bool

	cmd := &cobra.Command{
		Use:   "serve",
		Short: "Start the Wick daemon",
		RunE: func(cmd *cobra.Command, args []string) error {
			if !mcpMode {
				return fmt.Errorf("currently only --mcp mode is supported")
			}

			// MCP uses stdio for transport — logs must go to stderr only.
			slog.SetDefault(slog.New(slog.NewTextHandler(os.Stderr, &slog.HandlerOptions{Level: slog.LevelWarn})))

			ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt)
			defer cancel()

			cfg, err := engine.DefaultConfig()
			if err != nil {
				return err
			}
			eng := engine.New(cfg)
			defer eng.Close()

			return wickmcp.Serve(ctx, eng)
		},
	}

	cmd.Flags().BoolVar(&mcpMode, "mcp", false, "Run as MCP server on stdio")
	return cmd
}

func fetchCmd() *cobra.Command {
	var format string
	var noRobots bool

	cmd := &cobra.Command{
		Use:   "fetch <url>",
		Short: "Fetch a URL and print content to stdout",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			cfg, err := engine.DefaultConfig()
			if err != nil {
				return err
			}
			eng := engine.New(cfg)
			defer eng.Close()

			fetcher := fetch.NewFetcher(eng)
			result, err := fetcher.Fetch(fetch.Request{
				URL:           args[0],
				Format:        extract.Format(format),
				RespectRobots: !noRobots,
			})
			if err != nil {
				return err
			}

			if result.Title != "" {
				fmt.Fprintf(os.Stderr, "Title: %s\n", result.Title)
			}
			fmt.Fprintf(os.Stderr, "Status: %d | Time: %dms\n\n", result.StatusCode, result.TimingMs)
			fmt.Print(result.Content)
			return nil
		},
	}

	cmd.Flags().StringVar(&format, "format", "markdown", "Output format: markdown, html, text")
	cmd.Flags().BoolVar(&noRobots, "no-robots", false, "Ignore robots.txt restrictions")
	return cmd
}

func setupCmd() *cobra.Command {
	return &cobra.Command{
		Use:   "setup",
		Short: "Auto-configure MCP clients (Claude Code, Cursor)",
		RunE: func(cmd *cobra.Command, args []string) error {
			return setup.Setup()
		},
	}
}

func versionCmd() *cobra.Command {
	return &cobra.Command{
		Use:   "version",
		Short: "Print version information",
		Run: func(cmd *cobra.Command, args []string) {
			fmt.Printf("wick %s (commit: %s, built: %s)\n", version.Version, version.Commit, version.Date)
		},
	}
}
