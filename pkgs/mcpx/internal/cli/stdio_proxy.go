package cli

import (
	"context"
	"fmt"
	"io"
	"net"
	"os"

	"github.com/dezren39/mcpx/internal/config"
	"github.com/dezren39/mcpx/internal/daemon"
)

// CmdStdioProxy serves the daemon's API on stdin/stdout: it dials the
// local daemon's unix socket and pipes the bytes through.
//
// This is the remote half of an ssh:// endpoint (#1880). The client runs
// `ssh host mcpx stdio-proxy`, and the pipes of that ssh session become an
// HTTP connection to the daemon on this host. The socket path is resolved
// here, by this process, from its own defaults (and its own configuration
// file), so the client never needs to know how the remote filesystem is
// laid out. --sock exists for a host running more than one daemon, and
// ?sock= in the endpoint passes it through.
//
// It is a public, documented command rather than a hidden one: the ssh
// command line the client runs names it, so it is the first thing a user
// sees in a process list or an error, and it is useful on its own for
// wiring a tunnel by hand.
func (a *App) CmdStdioProxy(ctx context.Context, args []string) error {
	fs := newFlagSet("stdio-proxy")
	sock := fs.String("sock", "", "daemon socket on this host (default: mcpx's own path here)")
	cfgPath := fs.String("config", a.ConfigPath, "config file")
	if err := parseFlags(a, fs, args); err != nil {
		return err
	}

	path := *sock
	if path == "" {
		paths := a.Paths
		// A daemon keyed to a configuration file set listens on a keyed
		// socket (see Paths.ForConfig). Reading the same configuration
		// here keeps the proxy pointed at the daemon `mcpx daemon` on this
		// host would have started, which is what a zero-config client
		// expects. An unreadable config is not fatal: fall back to the
		// plain default socket rather than refusing the tunnel.
		if cfg, err := config.Load(*cfgPath); err == nil && len(cfg.Sources) > 0 {
			paths = paths.ForConfig(daemon.FingerprintConfig(cfg.Sources))
		}
		path = paths.Socket
	}

	var d net.Dialer
	conn, err := d.DialContext(ctx, "unix", path)
	if err != nil {
		return fmt.Errorf("stdio-proxy: no mcpx daemon at %s on this host: %w (start one with `mcpx daemon`, or point ?sock= at its socket)", path, err)
	}
	defer conn.Close()

	return pipeConn(conn, os.Stdin, os.Stdout)
}

// pipeConn copies in to conn and conn to out until the far end closes.
func pipeConn(conn io.ReadWriteCloser, in io.Reader, out io.Writer) error {
	// Requests flow one way down in and replies up out. When the far end
	// of the socket closes, out is done and so is this process; waiting
	// for in as well would hang on a terminal that never sends EOF.
	go func() {
		_, _ = io.Copy(conn, in)
		if cw, ok := conn.(interface{ CloseWrite() error }); ok {
			_ = cw.CloseWrite()
		}
	}()
	_, err := io.Copy(out, conn)
	return err
}
