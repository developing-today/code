// Command mcpx runs TypeScript against local MCP servers.
package main

import (
	"context"
	"os"
	"os/signal"
	"syscall"

	"github.com/dezren39/mcpx/internal/cli"
)

// version is overridden at build time with -ldflags "-X main.version=...".
// The default is a sentinel, not a release number (decision 0005 §3):
// the release version's single source is pkgs/mcpx/VERSION, and a
// binary built without ldflags must announce that it is not a release
// instead of impersonating one.
var version = "0.0.0-dev"

// main is only signal setup plus the in-process entrypoint (#1980,
// stage 1): everything it used to do inline -- global flags, dispatch,
// exit codes -- lives in cli.Run, where a test can drive it.
func main() {
	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()

	cli.Version = version
	os.Exit(cli.Run(ctx, os.Args[1:], cli.Env(os.Environ()), cli.Stdio{In: os.Stdin, Out: os.Stdout, Err: os.Stderr}))
}
