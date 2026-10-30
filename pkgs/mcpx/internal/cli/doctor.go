package cli

import (
	"context"
	"fmt"
	"github.com/dezren39/mcpx/internal/defaults"
	"github.com/dezren39/mcpx/internal/progspec"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"

	"github.com/dezren39/mcpx/internal/config"
	"github.com/dezren39/mcpx/internal/hooks"
	"github.com/dezren39/mcpx/internal/opencode"
)

// check is one diagnosis.
type check struct {
	Name   string `json:"name"`
	Status string `json:"status"` // ok, warn, fail
	Detail string `json:"detail,omitempty"`
	Fix    string `json:"fix,omitempty"`
}

// CmdDoctor diagnoses an installation.
//
// mcpx has grown enough moving parts -- a daemon, a runtime, a config chain,
// a log index, adapters, registries, a plugin -- that "it does not work" is
// no longer a question with one answer. Every one of these is something a
// person would otherwise check by hand, in an order they would have to know.
//
// Each check says what it found and what to do about it. A check that only
// reports a problem leaves the reader where they started.
func (a *App) CmdDoctor(ctx context.Context, args []string) error {
	fs := newFlagSet("doctor")
	verbose := fs.Bool("v", false, "show checks that passed")
	if err := parseFlags(a, fs, args); err != nil {
		return err
	}

	var checks []check
	add := func(c check) { checks = append(checks, c) }

	// Runtimes. Without one, nothing runs, and the failure otherwise appears
	// as a confusing exec error deep in a script invocation.
	found := []string{}
	for _, rt := range []string{"deno", "bun", "node"} {
		if p, err := exec.LookPath(rt); err == nil {
			found = append(found, rt+" ("+p+")")
		}
	}
	switch {
	case len(found) == 0:
		add(check{"runtime", "fail", "no deno, bun or node on PATH",
			"install one; deno is what mcpx is tested against"})
	case !strings.HasPrefix(found[0], "deno"):
		add(check{"runtime", "warn", strings.Join(found, ", "),
			"deno is preferred: it is the only runtime where --typecheck works"})
	default:
		add(check{"runtime", "ok", strings.Join(found, ", "), ""})
	}

	// The repo and worktree scopes, resolved from here. Native discovery
	// does not need git, so its absence is only reported when a repository
	// here needs it.
	//
	// A row either way: a check that disappears reads as a check that
	// passed, and "cannot tell" is the one answer doctor exists to give.
	if wd, err := os.Getwd(); err == nil {
		st, detail, fix := config.DiagnoseGit(wd)
		add(check{"git", st, detail, fix})
	} else {
		add(check{"git", "warn", "cannot read the working directory: " + err.Error(),
			"run from a directory that exists"})
	}

	// Settings. A contradiction here stops everything, so it is worth
	// reporting before anything else is attempted.
	if err := a.SettingsErr(); err != nil {
		add(check{"settings", "fail", err.Error(), "fix the configuration and run again"})
	} else {
		add(check{"settings", "ok", fmt.Sprintf("%d settings resolved",
			len(a.Settings().All())), ""})
	}
	if unknown := a.Settings().Unknown(); len(unknown) > 0 {
		var parts []string
		for _, u := range unknown {
			parts = append(parts, fmt.Sprintf("%s: %s", filepath.Base(u.File),
				strings.Join(u.Keys, ", ")))
		}
		add(check{"config keys", "warn", strings.Join(parts, "; "),
			"these are ignored; `mcpx config --schema` lists what is understood"})
	}

	// Configuration.
	cfg, cerr := config.Load(a.ConfigPath)
	config.ApplyPoolSettings(cfg, a.Settings())
	switch {
	case cerr != nil:
		add(check{"config", "fail", cerr.Error(), "fix the file or pass --config"})
	case cfg == nil || cfg.Path == "":
		add(check{"config", "warn", "no configuration file found",
			"run `mcpx init`, or `mcpx registry add <name> --write`"})
	default:
		if len(cfg.Ignored) > 0 {
			var parts []string
			for _, k := range cfg.Ignored {
				parts = append(parts, fmt.Sprintf("%s: %s.%s", filepath.Base(k.File), k.Server, k.Key))
			}
			add(check{"server keys", "warn", strings.Join(parts, "; "),
				"mcpx does not read these; another MCP host may. " +
					"plumbing.strictUnknownKeys refuses them"})
		}
		servers, rerr := cfg.ResolveAll()
		if rerr != nil {
			add(check{"config", "fail", rerr.Error(), "fix the server definitions"})
			break
		}
		add(check{"config", "ok", fmt.Sprintf("%s (%d servers)", cfg.Path, len(servers)), ""})

		// A server whose command is not installed is the single most common
		// cause of "mcpx does not work", and it is invisible until a call.
		var missing []string
		var nixSpecs int
		for _, s := range servers {
			if s.Command == "" {
				continue
			}
			if progspec.IsNix(s.Command) {
				// Resolved by nix at launch (progspec, #1921); a PATH
				// lookup would report it missing even on a machine
				// where it runs fine, and doctor does not build. A
				// malformed spec is still a config error here: it is
				// rejected at launch, so report it like one.
				if _, err := progspec.Parse(s.Command); err != nil {
					missing = append(missing, s.Name+": "+err.Error())
					continue
				}
				nixSpecs++
				continue
			}
			if _, err := exec.LookPath(s.Command); err != nil {
				missing = append(missing, s.Name+" needs "+s.Command)
			}
		}
		if len(missing) > 0 {
			add(check{"server commands", "fail", strings.Join(missing, "; "),
				"install them, or remove those servers from the configuration"})
		} else if len(servers) > 0 {
			detail := fmt.Sprintf("all %d runnable", len(servers))
			if nixSpecs > 0 {
				detail += fmt.Sprintf("; %d via nix: program specs, resolved to store paths at launch", nixSpecs)
			}
			add(check{"server commands", "ok", detail, ""})
		}

		// Auth posture, from the same redacted summary `mcpx config`
		// prints (#1926): which credential shape each server declares,
		// by variable reference only — doctor output gets pasted into
		// issues too, so the resolved value must never appear here.
		if len(servers) > 0 {
			var parts []string
			for _, s := range servers {
				parts = append(parts, fmt.Sprintf("%s: %s", s.Name, s.Auth.Describe()))
			}
			add(check{"auth", "ok", strings.Join(parts, "; "), ""})
		}
	}

	// Directories.
	for _, d := range []struct{ name, path string }{
		{"state", a.Paths.State},
		{"cache", a.Paths.Cache},
	} {
		if d.path == "" {
			continue
		}
		if err := os.MkdirAll(d.path, defaults.PublicDirMode); err != nil {
			add(check{d.name + " directory", "fail", err.Error(),
				"set paths." + d.name + " somewhere writable"})
			continue
		}
		probe := filepath.Join(d.path, ".mcpx-doctor")
		if err := os.WriteFile(probe, []byte("x"), defaults.PrivateMode); err != nil {
			add(check{d.name + " directory", "fail", d.path + " is not writable",
				"set paths." + d.name + " somewhere writable"})
			continue
		}
		_ = os.Remove(probe)
		add(check{d.name + " directory", "ok", d.path, ""})
	}

	// The socket path limit is a real constraint people hit with deep
	// directories, and the failure it produces is opaque.
	if a.Paths.State != "" {
		if sock := filepath.Join(a.Paths.State, "daemon-0123456789ab.sock"); len(sock) > 100 {
			add(check{"socket path", "warn",
				fmt.Sprintf("%d characters; the limit is about 104", len(sock)),
				"mcpx falls back to a private directory under the temp dir automatically"})
		}
	}

	// The daemon, and every way of reaching one. Which rung answered is the
	// thing somebody actually wants to know when a command is slow or is
	// talking to the wrong machine.
	dctx, cancel := context.WithTimeout(ctx, a.Settings().Duration("doctor.timeout"))
	defer cancel()
	_, ladder, lerr := a.Connect(dctx, ConnectOptions{
		Endpoint:   a.Settings().String("daemon.endpoint"),
		AllowSpawn: a.Settings().Bool("daemon.autostart"),
	})
	if lerr == nil {
		add(check{"connection", "ok", "reached over " + ladder.Chosen, ""})
	} else {
		add(check{"connection", "fail", strings.TrimSpace(ladder.Describe()),
			"run `mcpx daemon` in the foreground to see why it will not start"})
	}
	for _, at := range ladder.Attempts {
		if at.How == "sibling-socket" && at.Detail != "" {
			add(check{"other daemons", "warn", at.Detail,
				"each configuration gets its own; this is usually fine"})
		}
	}

	c, derr := a.ensure(dctx)
	if derr != nil {
		add(check{"daemon", "fail", derr.Error(),
			"run `mcpx daemon` in the foreground to see why it will not start"})
	} else {
		ns, nerr := c.Namespaces(dctx, a.Profile)
		if nerr != nil {
			add(check{"daemon", "warn", "running, but " + nerr.Error(), ""})
		} else {
			var broken []string
			for _, n := range ns {
				if n.Error != "" {
					broken = append(broken, n.Namespace+": "+firstLine(n.Error))
				}
			}
			add(check{"daemon", "ok", fmt.Sprintf("%d namespaces", len(ns)), ""})
			if len(broken) > 0 {
				add(check{"servers", "fail", strings.Join(broken, "; "),
					"`mcpx log --since 10m --level error` has the detail"})
			}
		}
	}

	// The log index, which is rebuildable and therefore only ever a warning.
	if st, serr := a.openStore(""); serr != nil {
		add(check{"log index", "warn", serr.Error(),
			"delete it; the JSONL files are the source of truth and it rebuilds"})
	} else {
		res, ierr := st.Ingest()
		if ierr != nil {
			add(check{"log index", "warn", ierr.Error(), "delete it; it rebuilds"})
		} else {
			detail := st.Path()
			if res.Skipped > 0 {
				add(check{"log index", "warn",
					fmt.Sprintf("%s (%d unparseable lines skipped)", detail, res.Skipped), ""})
			} else {
				add(check{"log index", "ok", detail, ""})
			}
		}
		_ = st.Close()
	}

	// Optional integrations. Absent is not a problem, so these are only
	// reported when present or when configured and broken.
	if path, oerr := opencode.Find(""); oerr == nil {
		add(check{"opencode database", "ok", path, ""})
	}
	if specs, aerr := a.loadAdapters(); aerr != nil {
		add(check{"adapters", "fail", aerr.Error(), "fix the declaration file"})
	} else if len(specs) > 0 {
		var bad []string
		for _, s := range specs {
			if err := s.Probe(); err != nil {
				bad = append(bad, err.Error())
			}
		}
		if len(bad) > 0 {
			add(check{"adapters", "warn", strings.Join(bad, "; "),
				"install the binaries, or remove those adapters"})
		} else {
			add(check{"adapters", "ok", fmt.Sprintf("%d declared", len(specs)), ""})
		}
	}
	if apis, aerr := a.loadAPIs(); aerr != nil {
		add(check{"api specs", "fail", aerr.Error(), "fix the declaration file"})
	} else if len(apis) > 0 {
		add(check{"api specs", "ok", fmt.Sprintf("%d declared", len(apis)), ""})
	}

	// Declared hooks. A hook whose declaration does not parse, or whose
	// executor cannot run, fails silently at fire time -- the daemon
	// logs it and carries on, which is right for the daemon and useless
	// for the person wondering why nothing happened. Doctor is where
	// that silence ends. (Runtime slow/failing counts live on the
	// daemon's hook runner and surface as hook.failed events.)
	if raw := a.Settings().String("hooks"); raw != "" {
		declared, herr := hooks.Parse(raw, a.Settings().Duration("hooks.timeout"))
		switch {
		case herr != nil:
			add(check{"hooks", "fail", herr.Error(), "fix the `hooks` setting; `mcpx hook ls` shows the same error"})
		default:
			var unready []string
			for _, h := range declared {
				if r := hookReady(h); r != "yes" {
					unready = append(unready, h.Name+": "+strings.TrimPrefix(r, "no: "))
				}
			}
			if len(unready) > 0 {
				add(check{"hooks", "warn", strings.Join(unready, "; "), "install the command or fix the module path"})
			} else {
				add(check{"hooks", "ok", fmt.Sprintf("%d declared, all executors ready", len(declared)), ""})
			}
		}
	}

	if a.JSON {
		return a.out(map[string]any{
			"checks":  checks,
			"version": a.Version,
			"platform": fmt.Sprintf("%s/%s go%s",
				runtime.GOOS, runtime.GOARCH, strings.TrimPrefix(runtime.Version(), "go")),
		})
	}
	return a.printChecks(checks, *verbose)
}

func (a *App) printChecks(checks []check, verbose bool) error {
	var failed, warned int
	for _, c := range checks {
		switch c.Status {
		case "fail":
			failed++
		case "warn":
			warned++
		case "ok":
			if !verbose {
				continue
			}
		}
		mark := map[string]string{"ok": "ok  ", "warn": "warn", "fail": "FAIL"}[c.Status]
		fmt.Printf("%s  %-18s %s\n", mark, c.Name, c.Detail)
		if c.Fix != "" {
			fmt.Printf("      %-18s %s\n", "", c.Fix)
		}
	}
	switch {
	case failed > 0:
		fmt.Printf("\n%d failed, %d warnings.\n", failed, warned)
		return fmt.Errorf("mcpx is not healthy")
	case warned > 0:
		fmt.Printf("\nNothing broken, %d warnings.\n", warned)
	default:
		fmt.Printf("\nAll %d checks passed.\n", len(checks))
	}
	return nil
}
