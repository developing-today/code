package cli

import (
	"path/filepath"
	"sync"

	"github.com/developing-today/code/pkgs/mcpx/internal/logging"
	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

// fileOptions builds the durable log's rotation policy from the resolved
// settings.
//
// The four knobs -- size, lines, age, retention -- were declared, documented
// and never passed anywhere, so every sink in the program ran on the built-in
// defaults and a user who set logging.maxBytes watched nothing change. They
// are read here, in one place, so the daemon's log and a script's log rotate
// by the same rule; a run whose records land in a file with a different
// retention from the daemon's is a log nobody can reason about.
func fileOptions(set *settings.Set, dir string) logging.FileOptions {
	return logging.FileOptions{
		Dir:      dir,
		MaxBytes: set.Bytes("logging.maxBytes"),
		MaxLines: int64(set.Int("logging.maxLines")),
		MaxAge:   set.Duration("logging.maxAge"),
		Keep:     set.Int("logging.keep"),
	}
}

// standaloneTaskAudit is the task-audit seam for a standalone MCP server
// (mcpx serve): its private task store has no daemon sink to borrow, so
// its audit records go to the same durable log the daemon's sink writes,
// under the same settings (logging.file / logging.dir / rotation). The
// sink opens lazily on the first record -- building the server (or
// printing its tools) creates no files -- and stays open for the
// process's lifetime, which is the server's lifetime. logging.file off,
// an unresolvable directory, or a sink failure leaves the seam inert:
// a missing audit log must never stop the server, exactly as the daemon
// treats a failed sink at startup.
func (a *App) standaloneTaskAudit() func(logging.Record) {
	var once sync.Once
	var sink *logging.FileSink
	return func(record logging.Record) {
		once.Do(func() {
			set := a.Settings()
			if !set.Bool("logging.file") {
				return
			}
			dir := firstNonEmpty(set.String("logging.dir"), filepath.Join(a.Paths.State, "logs"))
			if dir == "" {
				return
			}
			s, err := logging.NewFileSink(fileOptions(set, dir))
			if err != nil {
				return
			}
			sink = s
		})
		if sink != nil {
			sink.Write(record, nil)
		}
	}
}
