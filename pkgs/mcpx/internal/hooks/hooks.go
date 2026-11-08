// Package hooks is the declarative half of mcpx's hook vocabulary
// (docs/decisions/0001): hooks a user declares in configuration, in the
// `hooks` setting, that fire on events the daemon publishes to its bus.
//
// Every hook here is phase `observe` -- the only phase this package
// implements. An observe hook runs after the fact, asynchronously, and
// can never slow, veto or rewrite the operation it watched: the bus is
// fire-and-forget, the runner owns a bounded queue per hook, and every
// execution is cut by a hard timeout. The blocking phases (before,
// onSuccess, onError, suffix) belong to the enumerated hook points and
// are rejected at parse time rather than silently treated as observers.
//
// A hook is one action, exactly one per entry (decision 0001):
//
//   - run:    an argv array, never a shell string. Write
//     ["sh", "-c", "..."] to ask for a shell, out loud.
//   - module: a file run for its kind -- .py by the Python runtime,
//     .ts/.js/.mjs by the first available JS runtime (deno, bun,
//     node). The packet for #1872 names a Python module an executor
//     of its own; decision 0001 spells Python as a `run` until a
//     runtime knows it, and this is that knowledge, kept in one place.
//
// The event is delivered as JSON on stdin, with the flat fields also in
// the environment as MCPX_HOOK_* variables, so the common one-liner
// hook never has to parse JSON to know which server stopped.
package hooks

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"sort"
	"strings"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
	"github.com/developing-today/code/pkgs/mcpx/internal/events"
)

// Executor names what runs a hook.
type Executor string

const (
	ExecRun          Executor = "run"
	ExecModuleJS     Executor = "module:js"
	ExecModulePython Executor = "module:python"
)

// Hook is one validated declaration.
type Hook struct {
	Name string
	// On is the event filter: kind prefixes, verbatim events.Filter
	// semantics. "*" matches every hookable kind.
	On []string
	// When carries the filter's other fields.
	Server  string
	Session string
	URIs    []string
	// Phase is always "observe" in this package; Parse rejects the rest.
	Phase    string
	Executor Executor
	// Run is the argv for ExecRun.
	Run []string
	// Module is the file for the module executors.
	Module string
	// Timeout is the hard cut on one execution.
	Timeout time.Duration
	// FailureClosed is the hook's declared `"failure": "closed"`. In
	// the observe phase there is no operation left to refuse, so it
	// changes only how a failure is reported: closed failures are
	// counted separately and named in the hook.failed event, so a
	// guard-shaped hook's failures are never mistaken for noise.
	FailureClosed bool
}

// Filter is the hook's event filter, in the bus's own vocabulary.
func (h Hook) Filter() events.Filter {
	return events.Filter{Kinds: h.On, Server: h.Server, Session: h.Session, URIs: h.URIs}
}

// Matches reports whether the hook fires on e.
func (h Hook) Matches(e events.Event) bool { return h.Filter().Matches(e) }

// SubFilter is the filter the runner subscribes with: Filter, with any
// wildcard expanded to the hookable kinds, so the runner's own hook.*
// events are excluded BEFORE the bus buffers them for this subscription
// rather than only after dequeueing in work(). Buffered hook.failed
// events from other hooks otherwise occupy a busy wildcard hook's
// buffer, inflate its Stats.Dropped, and crowd out hookable events
// (Codex P2 on PR #2840). Non-wildcard patterns cannot match a hook.*
// kind — Parse's hookable check already rejects the ones that could —
// so only the wildcard (and an empty On, which the bus treats as
// match-all) needs expanding.
func (h Hook) SubFilter() events.Filter {
	f := h.Filter()
	kinds := make([]string, 0, len(f.Kinds))
	for _, k := range f.Kinds {
		if k == "*" {
			for _, hk := range HookableKinds() {
				kinds = append(kinds, string(hk))
			}
			continue
		}
		kinds = append(kinds, k)
	}
	if len(kinds) == 0 {
		for _, hk := range HookableKinds() {
			kinds = append(kinds, string(hk))
		}
	}
	f.Kinds = kinds
	return f
}

// decl is the JSON shape of one entry in the `hooks` setting.
type decl struct {
	Name   string          `json:"name"`
	On     json.RawMessage `json:"on"`
	Phase  string          `json:"phase"`
	Run    []string        `json:"run"`
	Module string          `json:"module"`
	When   struct {
		Server  string   `json:"server"`
		Session string   `json:"session"`
		URI     []string `json:"uri"`
	} `json:"when"`
	TimeoutMs int    `json:"timeoutMs"`
	Timeout   string `json:"timeout"`
	Failure   string `json:"failure"`
}

// HookableKinds is every kind a hook may subscribe to: every kind the
// bus carries, re-derived from the constants in internal/events, minus
// the ones that are never on the bus or must never retrigger a hook.
//
//   - resource.watching is a /v1/events stream report, never published.
//   - hook.failed is published by this package's runner; a hook on it
//     could retrigger itself forever.
func HookableKinds() []events.Kind {
	all := []events.Kind{
		events.ToolsChanged, events.ResourcesChanged, events.PromptsChanged,
		events.ResourceUpdated,
		events.ElicitOpened, events.ElicitAnswered, events.ElicitCompleted,
		events.SampleOpened, events.SampleAnswered,
		events.ServerStarted, events.ServerStopped,
		events.CallFinished,
		events.ServerLog, events.Progress,
		events.CatalogChanged,
	}
	sort.Slice(all, func(i, j int) bool { return all[i] < all[j] })
	return all
}

// hookable reports whether prefix p can match at least one hookable
// kind, under the Filter rule (exact, or prefix followed by a dot, or
// "*").
func hookable(p string) bool {
	if p == "*" {
		return true
	}
	if strings.HasPrefix(p, "hook.") || p == "hook" {
		return false
	}
	for _, k := range HookableKinds() {
		ks := string(k)
		if p == ks || strings.HasPrefix(ks, p+".") {
			return true
		}
	}
	return false
}

// Parse validates the raw `hooks` setting value: a JSON array of hook
// objects. defaultTimeout is the daemon's hooks.timeout, used by any
// hook that does not declare its own. An empty value declares no
// hooks and is not an error.
func Parse(raw string, defaultTimeout time.Duration) ([]Hook, error) {
	raw = strings.TrimSpace(raw)
	if raw == "" {
		return nil, nil
	}
	if defaultTimeout <= 0 {
		defaultTimeout = defaults.HooksTimeout
	}
	var decls []decl
	if err := json.Unmarshal([]byte(raw), &decls); err != nil {
		return nil, fmt.Errorf("hooks: not a JSON array of hook objects: %w", err)
	}
	out := make([]Hook, 0, len(decls))
	seen := map[string]bool{}
	for i, d := range decls {
		h, err := convert(d, i, defaultTimeout)
		if err != nil {
			return nil, err
		}
		if seen[h.Name] {
			return nil, fmt.Errorf("hooks: two hooks named %q", h.Name)
		}
		seen[h.Name] = true
		out = append(out, h)
	}
	return out, nil
}

func convert(d decl, i int, defaultTimeout time.Duration) (Hook, error) {
	h := Hook{
		Name:    d.Name,
		Phase:   d.Phase,
		Module:  strings.TrimSpace(d.Module),
		Server:  d.When.Server,
		Session: d.When.Session,
		URIs:    d.When.URI,
	}
	if h.Name == "" {
		h.Name = fmt.Sprintf("hook-%d", i+1)
	}
	if h.Phase == "" {
		h.Phase = "observe"
	}
	if h.Phase != "observe" {
		return h, fmt.Errorf("hooks: hook %q declares phase %q: only \"observe\" is implemented for config-declared hooks; the blocking phases belong to the hook points (docs/decisions/0001)", h.Name, h.Phase)
	}
	// on: one kind prefix, or a list of them.
	switch {
	case len(d.On) == 0:
		return h, fmt.Errorf("hooks: hook %q declares no \"on\": name at least one event kind prefix", h.Name)
	default:
		var one string
		if err := json.Unmarshal(d.On, &one); err == nil {
			h.On = []string{one}
		} else if err := json.Unmarshal(d.On, &h.On); err != nil {
			return h, fmt.Errorf("hooks: hook %q: \"on\" is neither a kind nor a list of kinds", h.Name)
		}
	}
	for _, p := range h.On {
		if !hookable(p) {
			return h, fmt.Errorf("hooks: hook %q: %q matches no hookable event kind (hookable: %s; resource.watching is never on the bus and hook.* is the runner's own)", h.Name, p, joinKinds())
		}
	}
	// Exactly one action.
	switch {
	case len(d.Run) > 0 && h.Module != "":
		return h, fmt.Errorf("hooks: hook %q declares both \"run\" and \"module\": a hook is exactly one action", h.Name)
	case len(d.Run) > 0:
		h.Executor = ExecRun
		h.Run = d.Run
	case h.Module != "":
		switch {
		case strings.HasSuffix(h.Module, ".py"):
			h.Executor = ExecModulePython
		case strings.HasSuffix(h.Module, ".ts"), strings.HasSuffix(h.Module, ".js"),
			strings.HasSuffix(h.Module, ".mjs"), strings.HasSuffix(h.Module, ".mts"):
			h.Executor = ExecModuleJS
		default:
			return h, fmt.Errorf("hooks: hook %q: module %q: cannot tell its kind from the extension (.py, .ts, .js, .mjs)", h.Name, h.Module)
		}
	default:
		return h, fmt.Errorf("hooks: hook %q declares no action: give \"run\" (an argv array) or \"module\" (a file)", h.Name)
	}
	// Timeout: per-hook wins, then the daemon default. Always bounded.
	h.Timeout = defaultTimeout
	if d.Timeout != "" {
		t, err := time.ParseDuration(d.Timeout)
		if err != nil {
			return h, fmt.Errorf("hooks: hook %q: timeout %q: %w", h.Name, d.Timeout, err)
		}
		h.Timeout = t
	}
	if d.TimeoutMs > 0 {
		h.Timeout = time.Duration(d.TimeoutMs) * time.Millisecond
	}
	if h.Timeout <= 0 {
		return h, fmt.Errorf("hooks: hook %q: timeout must be positive; a hook with no cut can hang its runner", h.Name)
	}
	switch d.Failure {
	case "", "open":
	case "closed":
		h.FailureClosed = true
	default:
		return h, fmt.Errorf("hooks: hook %q: failure %q: want \"open\" (the default) or \"closed\"", h.Name, d.Failure)
	}
	return h, nil
}

func joinKinds() string {
	ks := HookableKinds()
	parts := make([]string, len(ks))
	for i, k := range ks {
		parts[i] = string(k)
	}
	return strings.Join(parts, ", ")
}

// ArgvFor resolves a hook's action to a command line.
//
// run is verbatim: the declaration is the argv, never a shell string.
// A JS module runs under the first JS runtime on PATH, in the order
// the script settings try them (deno, bun, node). A Python module runs
// under python3, or `uv run` when python3 is absent -- Python is a
// module kind here because #1872 names it one, resolved in this one
// place so the choice is made once.
func ArgvFor(h Hook) ([]string, error) {
	switch h.Executor {
	case ExecRun:
		if len(h.Run) == 0 {
			return nil, fmt.Errorf("hook %s: empty run", h.Name)
		}
		return h.Run, nil
	case ExecModulePython:
		if p, err := exec.LookPath("python3"); err == nil {
			return []string{p, h.Module}, nil
		}
		if p, err := exec.LookPath("uv"); err == nil {
			return []string{p, "run", h.Module}, nil
		}
		return nil, fmt.Errorf("hook %s: no python3 or uv on PATH for module %s", h.Name, h.Module)
	case ExecModuleJS:
		for _, rt := range []string{"deno", "bun", "node"} {
			p, err := exec.LookPath(rt)
			if err != nil {
				continue
			}
			if rt == "deno" {
				return []string{p, "run", "-A", h.Module}, nil
			}
			return []string{p, h.Module}, nil
		}
		return nil, fmt.Errorf("hook %s: no deno, bun or node on PATH for module %s", h.Name, h.Module)
	}
	return nil, fmt.Errorf("hook %s: unknown executor %q", h.Name, h.Executor)
}

// Ready reports whether the hook's executor can run at all on this
// machine -- "yes" if ready, or "no: <reason>" explaining why it cannot.
func Ready(h Hook) string {
	switch h.Executor {
	case ExecRun:
		if len(h.Run) == 0 {
			return "no: empty run"
		}
		if _, err := exec.LookPath(h.Run[0]); err != nil {
			return "no: " + h.Run[0] + " not on PATH"
		}
	case ExecModulePython:
		if _, err := os.Stat(h.Module); err != nil {
			return "no: module file missing"
		}
		if _, err := exec.LookPath("python3"); err == nil {
			return "yes"
		}
		if _, err := exec.LookPath("uv"); err == nil {
			return "yes"
		}
		return "no: python3 or uv not on PATH"
	case ExecModuleJS:
		if _, err := os.Stat(h.Module); err != nil {
			return "no: module file missing"
		}
		for _, rt := range []string{"deno", "bun", "node"} {
			if _, err := exec.LookPath(rt); err == nil {
				return "yes"
			}
		}
		return "no: deno, bun or node not on PATH"
	}
	return "yes"
}
