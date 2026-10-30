package cli

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"text/tabwriter"
	"time"

	"github.com/dezren39/mcpx/internal/hooks"
)

// CmdHook inspects the config-declared hooks (the `hooks` setting,
// docs/decisions/0001). Listing is the debuggability surface the issue
// asked for; a runtime-mutable hook table belongs to the privileged
// plugin surface and is deliberately not here.
//
//	mcpx hook ls    the hooks in force, where each came from, and
//	                whether its executor can actually run
func (a *App) CmdHook(ctx context.Context, args []string) error {
	if len(args) == 0 || args[0] != "ls" {
		return fmt.Errorf("usage: mcpx hook ls -- list the declared hooks in force (the only subcommand; the hook table is config, not runtime state)")
	}
	fs := newFlagSet("hook")
	if err := parseFlags(a, fs, args[1:]); err != nil {
		return err
	}
	declared, err := hooks.Parse(a.Settings().String("hooks"), a.Settings().Duration("hooks.timeout"))
	if err != nil {
		return err
	}
	type row struct {
		Name     string `json:"name"`
		On       string `json:"on"`
		Executor string `json:"executor"`
		Source   string `json:"source"`
		Timeout  string `json:"timeout"`
		Failure  string `json:"failure"`
		Ready    string `json:"ready"`
	}
	rows := make([]row, 0, len(declared))
	for _, h := range declared {
		src := strings.Join(h.Run, " ")
		if h.Module != "" {
			src = h.Module
		}
		fail := "open"
		if h.FailureClosed {
			fail = "closed"
		}
		rows = append(rows, row{
			Name: h.Name, On: strings.Join(h.On, ","), Executor: string(h.Executor),
			Source: src, Timeout: h.Timeout.Round(time.Millisecond).String(),
			Failure: fail, Ready: hookReady(h),
		})
	}
	if a.JSON {
		return a.out(rows)
	}
	if len(rows) == 0 {
		fmt.Println("No hooks declared. Declare them in the `hooks` setting:")
		fmt.Println(`  "hooks": [{"name": "page", "on": "server.stopped", "run": ["notify-send", "done"]}]`)
		return nil
	}
	tw := tabwriter.NewWriter(os.Stdout, 0, 0, 2, ' ', 0)
	fmt.Fprintln(tw, "NAME\tON\tEXECUTOR\tSOURCE\tTIMEOUT\tFAILURE\tREADY")
	for _, r := range rows {
		fmt.Fprintf(tw, "%s\t%s\t%s\t%s\t%s\t%s\t%s\n",
			r.Name, r.On, r.Executor, r.Source, r.Timeout, r.Failure, r.Ready)
	}
	return tw.Flush()
}

// hookReady reports whether the hook's executor can run at all on this
// machine -- the check that turns "the hook never fired" from a mystery
// into a line of output.
func hookReady(h hooks.Hook) string {
	switch h.Executor {
	case hooks.ExecRun:
		if len(h.Run) == 0 {
			return "no: empty run"
		}
		if _, err := exec.LookPath(h.Run[0]); err != nil {
			return "no: " + h.Run[0] + " not on PATH"
		}
	case hooks.ExecModuleJS, hooks.ExecModulePython:
		if _, err := os.Stat(h.Module); err != nil {
			return "no: module file missing"
		}
	}
	return "yes"
}
