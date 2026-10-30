package cli

import (
	"context"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"strings"

	"github.com/dezren39/mcpx/internal/config"
	"github.com/dezren39/mcpx/internal/daemon"
)

// Version is the build version Run reports. cmd/mcpx sets it from its
// -ldflags-overridden var before calling Run, so the shipped binary and
// an in-process caller stamp the same string.
var Version = "0.1.0"

// Env is a process environment as "key=value" entries, the shape
// os.Environ() returns. Run takes it explicitly so an in-process caller
// states the environment it means instead of inheriting whatever the
// host process happens to carry.
//
// Stage 1 of #1980: Run records the Env on the App (see App.Environ),
// but the internals still read os.Getenv, so in-process tests set values
// with t.Setenv and must not use t.Parallel. Stage 2 threads this Env
// through ResolvePaths, settings.Environ and the plumbing source.
type Env []string

// Get returns the value for key, or "" when it is absent.
func (e Env) Get(key string) string {
	v, _ := e.Lookup(key)
	return v
}

// Lookup returns the value for key and whether it was present. The first
// entry wins, matching os.Getenv.
func (e Env) Lookup(key string) (string, bool) {
	prefix := key + "="
	for _, kv := range e {
		if strings.HasPrefix(kv, prefix) {
			return strings.TrimPrefix(kv, prefix), true
		}
	}
	return "", false
}

// Stdio is the streams a Run invocation reads and writes. A nil field
// falls back to the matching os stream, so the zero value behaves
// exactly like the binary. Stage 1: Run's own dispatch-level output
// (version, unknown command/flag, handler errors) goes to these
// streams; commands still write to the os streams directly until the
// threading in stage 2 lands.
type Stdio struct {
	In  io.Reader
	Out io.Writer
	Err io.Writer
}

func (s Stdio) out() io.Writer {
	if s.Out != nil {
		return s.Out
	}
	return os.Stdout
}

func (s Stdio) err() io.Writer {
	if s.Err != nil {
		return s.Err
	}
	return os.Stderr
}

// Environ returns the environment this App was invoked with: the Env
// Run was given, or os.Environ() when the App was built directly.
func (a *App) Environ() Env {
	if a.environ != nil {
		return a.environ
	}
	return Env(os.Environ())
}

// splitList accepts comma or space separated values.
func splitList(s string) []string {
	return strings.FieldsFunc(s, func(r rune) bool { return r == ',' || r == ' ' })
}

// looksRunnable reports whether a bare first argument should be run, and as
// what. It is deliberately conservative: a bare word that is not a file is
// left to fail as an unknown command, because guessing that `mcpx serach` was
// a snippet would replace a clear error with a baffling one.
func looksRunnable(arg string) string {
	if arg == "" || strings.HasPrefix(arg, "-") {
		return ""
	}
	for _, ext := range []string{".ts", ".js", ".mts", ".mjs"} {
		if strings.HasSuffix(arg, ext) {
			return "file"
		}
	}
	// Deliberately narrow. An "=" alone is not evidence of source -- a
	// mistyped flag has one -- and treating it as such would run a typo as a
	// program. These markers do not occur in a command name.
	if strings.ContainsAny(arg, "(){};") || strings.Contains(arg, "await ") ||
		strings.Contains(arg, "console.") || strings.Contains(arg, "tools.") ||
		strings.Contains(arg, "=>") {
		return "source"
	}
	if st, err := os.Stat(arg); err == nil && !st.IsDir() {
		return "file"
	}
	return ""
}

// Run is the in-process entrypoint for the whole CLI (#1980, approach C,
// stage 1). It owns everything main() used to do inline -- global-flag
// parsing, building the App, dispatch through the Handlers table, and
// turning handler errors into exit codes -- and returns the exit code
// instead of calling os.Exit, so a test can drive the same code path a
// user takes. cmd/mcpx is now only signal setup plus
// os.Exit(cli.Run(...)).
//
// The binary suite remains the superset: signals, real exit codes as a
// process boundary, and argv as the shell delivers it only exist there.
func Run(ctx context.Context, args []string, env Env, stdio Stdio) int {
	app := &App{Version: Version, Paths: daemon.ResolvePaths(), environ: env}
	// Script resolution is reached from places that do not carry an App, so
	// it is told once where to read settings from. The plumbing source is a
	// package global (clientSettings consults it too), so scope it to this
	// invocation like the paths-config seed below: an in-process caller
	// would otherwise leak its settings -- its env layer included -- into
	// every later client built in the same process. That leak is what the
	// stress suite caught: a Run test setting MCPX_DAEMON_AUTOSTART=false
	// left later EnsureDaemon tests reading autostart as off.
	prevPlumbing := plumbingApp
	SetPlumbingSource(app)
	defer SetPlumbingSource(prevPlumbing)

	// The --paths-config seed is a package global in config (it has to
	// be captured before any settings exist), so an in-process caller
	// would otherwise inherit the previous invocation's seed. Scope it
	// to this invocation: clear on entry, and clear again on the way
	// out so nothing leaks into the next call.
	config.SetPathsConfigSeed(nil)
	defer config.SetPathsConfigSeed(nil)

	// An in-process daemon lives exactly as long as this command. Stopped on
	// every exit path, including an error, or its servers would outlive the
	// invocation that owns them. (In main() this only guarded the dispatch
	// path, because the process was exiting anyway; in-process it must
	// guard every return.)
	defer app.CloseInline()

	// Global flags may appear before the subcommand.
	for len(args) > 0 && strings.HasPrefix(args[0], "-") {
		switch {
		case args[0] == "--json":
			app.JSON = true
			args = args[1:]
		case args[0] == "--tui":
			// A global flag as well as a subcommand, because it is as likely
			// to be reached for mid-command ("...actually, show me") as
			// chosen up front.
			if err := app.CmdTUI(ctx, nil); err != nil {
				fmt.Fprintln(stdio.err(), "mcpx:", err)
				return 1
			}
			return 0
		case args[0] == "--profile" && len(args) > 1:
			app.Profile.Names = append(app.Profile.Names, splitList(args[1])...)
			args = args[2:]
		case strings.HasPrefix(args[0], "--profile="):
			app.Profile.Names = append(app.Profile.Names, splitList(strings.TrimPrefix(args[0], "--profile="))...)
			args = args[1:]
		case args[0] == "--skip-default":
			app.Profile.SkipDefault = true
			args = args[1:]
		case args[0] == "--all-profiles":
			app.Profile.All = true
			args = args[1:]
		case args[0] == "--config" && len(args) > 1:
			app.ConfigPath = args[1]
			args = args[2:]
		case strings.HasPrefix(args[0], "--config="):
			app.ConfigPath = strings.TrimPrefix(args[0], "--config=")
			args = args[1:]
		case args[0] == "--paths-config" && len(args) > 1:
			// A bootstrap flag like --config: it decides which files
			// the settings are read from, so it is captured here,
			// before any settings exist (issue #1842).
			config.SetPathsConfigSeed(splitList(args[1]))
			args = args[2:]
		case strings.HasPrefix(args[0], "--paths-config="):
			config.SetPathsConfigSeed(splitList(strings.TrimPrefix(args[0], "--paths-config=")))
			args = args[1:]
		case args[0] == "--version" || args[0] == "-v":
			fmt.Fprintln(stdio.out(), "mcpx", Version)
			return 0
		case args[0] == "--help" || args[0] == "-h":
			_ = app.CmdHelp(ctx, nil)
			return 0
		default:
			// Any setting every command accepts is also a global flag, so
			// `mcpx --plumbing-strict-unknown-keys ls` means what
			// `mcpx ls --plumbing-strict-unknown-keys` does.
			rest, err := app.ApplyGlobalSettingFlags(args)
			if err != nil {
				fmt.Fprintln(stdio.err(), "mcpx:", err)
				return 2
			}
			if len(rest) < len(args) {
				args = rest
				continue
			}
			fmt.Fprintf(stdio.err(), "unknown global flag %q\n", args[0])
			return 2
		}
	}

	if len(args) == 0 {
		// Bare `mcpx` opens the browser when there is a terminal to draw on,
		// and prints help when there is not. Help was the old behaviour and
		// is still the right answer for a pipe; for a person at a prompt,
		// a wall of usage is a worse first impression than the thing itself.
		if err := app.CmdTUI(ctx, nil); err != nil {
			_ = app.CmdHelp(ctx, nil)
		}
		return 0
	}

	cmd, rest := args[0], args[1:]
	// The table lives in this package, beside Commands(), so a test can
	// hold the two together; in main it drifted from the declared list
	// by seven.
	handlers := app.Handlers()

	h, ok := handlers[cmd]
	if !ok {
		// An argument that is obviously a script or a snippet runs, rather
		// than being refused for not being a subcommand. The alternative --
		// insisting on `mcpx run` -- rejects the most natural thing to type
		// in order to protect a namespace that has no collisions in it.
		if kind := looksRunnable(cmd); kind != "" {
			if kind == "file" {
				h, rest = app.CmdRun, args
			} else {
				h, rest = app.CmdExec, args
			}
		} else {
			fmt.Fprintf(stdio.err(), "unknown command %q\n\n", cmd)
			_ = app.CmdHelp(ctx, nil)
			return 2
		}
	}

	// Settings are resolved before dispatch, so a contradiction in a config
	// file is reported once, up front, rather than by whichever command
	// happens to read the offending value first.
	if err := app.SettingsErr(); err != nil {
		fmt.Fprintln(stdio.err(), "mcpx:", err)
		return 2
	}

	if err := h(ctx, rest); err != nil {
		// Help was asked for and already printed; it is not a failure.
		// Commands reached without the dispatch table (a script file named
		// on the command line) can still return this directly.
		if errors.Is(err, flag.ErrHelp) {
			return 0
		}
		var ec ExitCoder
		if errors.As(err, &ec) {
			// A script's exit status is not a failure of mcpx itself:
			// the run already reported itself, so there is nothing to
			// prefix with "mcpx:" -- only the status carries over.
			var se *exitStatusError
			if !errors.As(err, &se) {
				fmt.Fprintln(stdio.err(), "mcpx:", err)
			}
			return ec.ExitCode()
		}
		if errors.Is(err, ErrNoDaemon) {
			fmt.Fprintln(stdio.err(), "mcpx: daemon is not running and could not be started")
		} else {
			fmt.Fprintln(stdio.err(), "mcpx:", err)
		}
		return 1
	}
	return 0
}

// exitStatusError carries a script run's exit status back to Run as an
// error, so a handler never has to os.Exit to report it: in-process, an
// os.Exit would terminate the embedding application or test runner (and
// skip Run's deferred CloseInline); as a returned ExitCoder, Run turns
// it into the same process exit code at the one place that owns it.
type exitStatusError struct{ code int }

func (e *exitStatusError) Error() string { return fmt.Sprintf("exit status %d", e.code) }
func (e *exitStatusError) ExitCode() int { return e.code }
