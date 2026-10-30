package cli

import (
	"context"
	"errors"
	"fmt"

	"github.com/dezren39/mcpx/internal/mcpclient"
)

// CmdSupervise runs one stdio MCP server child under a supervisor
// (#1385, Phase 1 of #1384). It is the re-exec target of the supervised
// transport: the daemon spawns it detached, it owns the child's pipes
// for the child's whole life, and daemons attach to its unix socket
// one at a time. The supervisor process's environment and working
// directory are the child's — the spawning transport sets them — so
// the child is spawned with InheritEnv and no extra env of its own.
//
// Usage: mcpx supervise --socket <path> -- <command> [args...]
func (a *App) CmdSupervise(ctx context.Context, args []string) error {
	var socket string
	var child []string
	for i := 0; i < len(args); i++ {
		switch {
		case args[i] == "--socket" && i+1 < len(args):
			socket = args[i+1]
			i++
		case args[i] == "--":
			child = args[i+1:]
			i = len(args)
		default:
			return fmt.Errorf("supervise: unexpected argument %q (want --socket <path> -- <command> [args...])", args[i])
		}
	}
	if socket == "" {
		return errors.New("supervise: --socket is required")
	}
	if len(child) == 0 {
		return errors.New("supervise: no child command after --")
	}
	return mcpclient.RunSupervisor(mcpclient.SupervisorOptions{
		SocketPath: socket,
		Command:    child[0],
		Args:       child[1:],
		InheritEnv: true,
	})
}
