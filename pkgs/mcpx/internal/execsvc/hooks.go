package execsvc

// Hooks: deterministic code at the edges of a run.
//
// The five `script.*` phases already run lines inside a file script's launcher,
// for the CLI and for config. This is the same idea at the `script` point of a
// run, over the wire: a hook is a whole script run by mcpx's own runner, in the
// same content-addressed workdir, with the same runtime and the same
// permissions as the script it wraps, so a hook can reach the tools the script
// could reach and cannot reach more.
//
// The vocabulary is docs/decisions/0001-hook-vocabulary.md's, not the one this
// packet was written with: its `pre`/`post` became `before`/`onSuccess`, `main`
// is not a hook at all, and a `hook` frame is placed so that the result a
// consumer reads is the real one.

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/dezren39/mcpx/internal/diagnose"
	"github.com/dezren39/mcpx/internal/logging"
	"github.com/dezren39/mcpx/internal/runner"
)

// Hook phases at the `script` point, in the order they run.
const (
	// HookBefore runs before the script is started. It may advise; it cannot
	// change the outcome, because in this version a `before` hook has no
	// effect above advise (a refusing before is a policy mechanism, #185).
	HookBefore = "before"
	// HookOnSuccess runs after the script returned without error and before
	// the result is emitted. It may replace the result value.
	HookOnSuccess = "onSuccess"
	// HookOnError runs after the script failed and before its error is
	// emitted. It may replace the error with a result -- the one place this
	// deliberately differs from the shipped `script.onError`, which always
	// rethrows.
	HookOnError = "onError"
	// HookSuffix runs on both paths, after the result or error frame, like a
	// finally. It cannot change the outcome.
	HookSuffix = "suffix"
)

// HookPointScript is the point every hook in this version attaches to.
const HookPointScript = "script"

// hookActionModule is what a hook is recorded as in its frame: a script run
// by mcpx's own runner, per 0001's action table.
const hookActionModule = "module"

// hookTimeout is a hook's own budget.
//
// Its own rather than the script's: a slow hook must not eat the budget of the
// script it was meant to wrap, and a hook that hangs must not hold the whole
// run open. Bounded generously, because a hook that shells out to install
// something is a legitimate thing to ask for.
const hookTimeout = 60 * time.Second

// Hook is one entry in the hooks list: a phase and the script that runs in it.
//
// `Script` is either a path to a file, or inline TypeScript, told apart the way
// a person would: something that is one line, ends in a script extension and
// exists is a file; anything else is source.
type Hook struct {
	Phase  string `json:"phase"`
	Script string `json:"script"`
	// Name labels the hook in errors and frames. Optional.
	Name string `json:"name,omitempty"`
}

// HookFrame is what a hook reports about itself.
//
// It carries what 0001 asks for -- on, phase, action, duration and whether the
// hook changed anything -- plus the original when it did, so a caller can
// always see what mcpx changed.
type HookFrame struct {
	On     string `json:"on"`
	Phase  string `json:"phase"`
	Action string `json:"action,omitempty"`
	// Effect is what the hook was allowed to do, which is fixed by its phase
	// in this version rather than declared per hook.
	Effect     string          `json:"effect,omitempty"`
	DurationMs int64           `json:"durationMs,omitempty"`
	Changed    bool            `json:"changed"`
	Original   json.RawMessage `json:"original,omitempty"`
	// Dropped names the keys an output carried that its phase may not
	// honour. Reported rather than silently ignored.
	Dropped []string `json:"dropped,omitempty"`
	Advice  string   `json:"advice,omitempty"`
	Error   string   `json:"error,omitempty"`
}

// hookContext is the typed context a hook receives on standard input.
type hookContext struct {
	RunID  string `json:"runId"`
	Point  string `json:"point"`
	Phase  string `json:"phase"`
	Result any    `json:"result,omitempty"`
	Error  string `json:"error,omitempty"`
	// Diagnostic is the same structured shape an error carries elsewhere in
	// mcpx, so a hook can branch on a kind instead of matching prose.
	Diagnostic *diagnose.Diagnostic `json:"diagnostic,omitempty"`
	ExitCode   int                  `json:"exitCode"`
	DurationMs int64                `json:"durationMs"`
	Artifacts  []Artifact           `json:"artifacts"`
}

// hookOutput is what a hook may return: one JSON object on stdout.
//
// The keys are the dial's levels (0002). Only two are honoured here, because a
// deterministic hook's output is source-level advice or a replacement value;
// `run` and `ask` are the prompt half and are not implemented.
type hookOutput struct {
	Apply  json.RawMessage `json:"apply"`
	Advise string          `json:"advise"`
}

// effectFor reports what a phase is allowed to change.
func effectFor(phase string) string {
	switch phase {
	case HookOnSuccess, HookOnError:
		return "apply"
	default:
		return "advise"
	}
}

// validateHooks refuses a hooks list mcpx cannot honour, naming the entry.
func validateHooks(hooks []Hook, origin string) error {
	for i, h := range hooks {
		switch h.Phase {
		case HookBefore, HookOnSuccess, HookOnError, HookSuffix:
		default:
			return fmt.Errorf("exec: %s hook %d: unknown phase %q (one of %s, %s, %s, %s)",
				origin, i, h.Phase, HookBefore, HookOnSuccess, HookOnError, HookSuffix)
		}
		if strings.TrimSpace(h.Script) == "" {
			return fmt.Errorf("exec: %s hook %d (%s): script is empty", origin, i, h.Phase)
		}
	}
	return nil
}

// hooksFor is the list in force: config hooks first, then the per-call ones.
//
// Appending rather than replacing is the layering `ScriptConfig.Prefix` uses,
// and the order is the order they run in: the habit declared once for a
// project runs before the exception declared for one call.
func (s *Service) hooksFor(call []Hook) []Hook {
	if len(s.Hooks) == 0 {
		return call
	}
	out := make([]Hook, 0, len(s.Hooks)+len(call))
	out = append(out, s.Hooks...)
	return append(out, call...)
}

// phaseHooks selects the hooks for one phase, in declaration order.
func phaseHooks(hooks []Hook, phase string) []Hook {
	var out []Hook
	for _, h := range hooks {
		if h.Phase == phase {
			out = append(out, h)
		}
	}
	return out
}

// hookRun is one completed hook execution.
type hookRun struct {
	Hook
	Out      hookOutput
	Frame    HookFrame
	Duration time.Duration
	// Failed is true when the hook itself did not run or did not produce a
	// JSON object. It is a result, not an error: the run carries on.
	Failed bool
}

// runHooks executes every hook of one phase and returns what they produced.
//
// The frames are emitted here rather than by the caller because placement is
// part of the contract: a caller reads a hook frame and knows exactly how far
// the run had got.
func (s *Service) runHooks(ctx context.Context, phase string, hooks []Hook, hctx hookContext,
	ropts runner.Options, res *Result, emit func(Frame)) []hookRun {

	var runs []hookRun
	for _, h := range hooks {
		// The context is refreshed from the current outcome before every
		// hook, so a transform sees what the hook before it produced and
		// transformations compose in declaration order rather than every
		// hook reading the outcome the script itself left.
		hctx.Result, hctx.Error = res.Result, res.Error
		hctx.Diagnostic = hookDiagnostic(res.Error)
		hctx.ExitCode = res.ExitCode
		hctx.Artifacts = res.Artifacts
		run := s.runOneHook(ctx, phase, h, hctx, ropts, res, emit)
		run.Frame.Effect = effectFor(phase)
		run.Frame.Original = applyHookResult(&run, phase, res)
		run.Frame.Changed = run.Frame.Original != nil
		f := run.Frame
		emit(Frame{Type: FrameHook, RunID: res.RunID, Hook: &f})
		runs = append(runs, run)
	}
	return runs
}

// runOneHook starts the hook's script and reads its answer.
func (s *Service) runOneHook(ctx context.Context, phase string, h Hook, hctx hookContext,
	ropts runner.Options, res *Result, emit func(Frame)) hookRun {

	out := hookRun{Hook: h}
	out.Frame = HookFrame{On: HookPointScript, Phase: phase, Action: hookActionModule}

	body, err := json.Marshal(hctx)
	if err != nil {
		out.Failed = true
		out.Frame.Error = "hook context: " + err.Error()
		return out
	}

	opts := ropts
	// The hook gets the same runtime, permissions, working directory and
	// content-addressed client as the script, and nothing of the parent's
	// phases: a hook running the parent's onSuccess would recurse.
	opts.Phases = runner.Phases{}
	opts.Launcher, opts.LauncherName, opts.AllowRepeat = "", "", nil
	opts.Export, opts.Args = "", nil
	opts.TypeCheck = "off"
	opts.Stdin = strings.NewReader(string(body))
	opts.Env = copyEnv(ropts.Env)
	opts.Env["MCPX_HOOK_PHASE"] = phase
	opts.Env["MCPX_HOOK_POINT"] = HookPointScript
	if hctx.Error != "" {
		// The one-line case: a hook that only wants to say what went wrong
		// should not have to parse stdin for it. Truncated, because an
		// environment variable is not a document.
		msg := hctx.Error
		if len(msg) > 1024 {
			msg = msg[:1024]
		}
		opts.Env["MCPX_HOOK_ERROR"] = msg
	}
	if res.RunID != "" {
		opts.Env["MCPX_RUN"] = res.RunID
	}
	var stdout strings.Builder
	opts.Stdout = &stdout
	stderr := &tailBuffer{limit: 4096}
	opts.Stderr = stderr
	opts.CollectLogs = func(r logging.Record) {
		rec := LogRecord{Time: r.Time.Format(time.RFC3339Nano), Level: logging.LevelName(r.Level),
			Message: r.Msg, Attrs: r.Attrs}
		res.Logs = append(res.Logs, rec)
		if s.Logs != nil {
			s.Logs(r)
		}
		emit(Frame{Type: FrameLog, Level: rec.Level, Message: rec.Message, Attrs: rec.Attrs})
	}
	opts.OnResult = func(v logging.Streamed) {
		res.Emits = append(res.Emits, v.Value)
		emit(Frame{Type: FrameEmit, Value: v.Value})
	}
	// The run's own artifact handler stays: an artifact a hook produces is
	// an artifact of the run, recorded and framed the same way. RunWith
	// reconciles and delivers after the post-execution hooks, so a hook's
	// artifacts reach the caller like the script's own.
	opts.OnArtifact = ropts.OnArtifact
	opts.Timeout = hookTimeout

	// A hook named as a file is run as a file, so it can be the ordinary
	// `ensure-deps.ts` a person would have written anyway; anything else is
	// source, with the same prelude a snippet gets so `tools` is in scope.
	script := strings.TrimSpace(h.Script)
	if p, ok := s.hookFile(script, opts.Dir); ok {
		opts.File, opts.Source = p, ""
	} else {
		// Source alone: runner.Run prepends opts.Prelude itself, so folding
		// the prelude in here would bind the generated imports twice and
		// every inline hook would fail with a syntax error.
		opts.File, opts.Source = "", script+"\n"
	}

	hctxCtx, cancel := context.WithTimeout(ctx, hookTimeout)
	defer cancel()
	start := time.Now()
	rres, rerr := runner.Run(hctxCtx, opts)
	out.Duration = time.Since(start)
	out.Frame.DurationMs = out.Duration.Milliseconds()
	if rres != nil && rres.Duration > 0 {
		out.Frame.DurationMs = rres.Duration.Milliseconds()
	}

	switch {
	case rerr != nil:
		out.Failed = true
		out.Frame.Error = rerr.Error()
	case rres == nil:
		out.Failed = true
		out.Frame.Error = "the hook did not start"
	case rres.ExitCode != 0:
		// A non-zero exit is a result, not an error: the hook failed, the
		// frame says so, and the operation carries on (0001).
		out.Failed = true
		msg := strings.TrimSpace(stripANSI(stderr.String()))
		if msg == "" {
			msg = fmt.Sprintf("hook exited %d", rres.ExitCode)
		}
		out.Frame.Error = msg
	case rres.TimedOut:
		out.Failed = true
		out.Frame.Error = "the hook timed out"
	}

	if !out.Failed {
		text := strings.TrimSpace(stdout.String())
		if text != "" {
			if err := json.Unmarshal([]byte(text), &out.Out); err != nil {
				// Not an object is not a failure: a hook that printed a
				// sentence advised, and saying so is more useful than
				// refusing the answer it did give.
				out.Out = hookOutput{Advise: text}
			}
		}
		out.Frame.Advice = out.Out.Advise
	}
	return out
}

// applyHookResult honours what a hook may change in its phase.
//
// It returns the original value when it replaced one, so the frame can carry
// what the caller would otherwise have received.
func applyHookResult(r *hookRun, phase string, res *Result) json.RawMessage {
	if r.Failed || len(r.Out.Apply) == 0 {
		return nil
	}
	if effectFor(phase) != "apply" {
		r.Frame.Dropped = append(r.Frame.Dropped, "apply")
		return nil
	}
	var v any
	if err := json.Unmarshal(r.Out.Apply, &v); err != nil {
		r.Frame.Dropped = append(r.Frame.Dropped, "apply")
		r.Frame.Error = "apply is not JSON: " + err.Error()
		return nil
	}
	original, _ := json.Marshal(res.Result)
	switch phase {
	case HookOnSuccess:
		res.Result = v
	case HookOnError:
		// The original outcome on this path is the error, not the result
		// value (usually nil for a thrown exception): record it before it
		// is cleared, or the frame claims a replacement happened while
		// showing the consumer nothing of what was replaced.
		if res.Error != "" {
			original, _ = json.Marshal(map[string]any{"error": res.Error})
		}
		res.Result = v
		res.Error = ""
		res.ExitCode = 0
	}
	return original
}

// hookDiagnostic is the diagnostic a hook receives, kept small: the kind of
// thing that went wrong and what the run said about it.
func hookDiagnostic(msg string) *diagnose.Diagnostic {
	if msg == "" {
		return nil
	}
	return &diagnose.Diagnostic{Kind: diagnose.Kind("exec-error"), Message: msg}
}

// hookFile reports whether a hook's script names a file rather than being
// source: one line, a script extension, and it exists beside the run.
func (s *Service) hookFile(script, dir string) (string, bool) {
	if script == "" || strings.ContainsAny(script, "\n;={}") {
		return "", false
	}
	switch filepath.Ext(script) {
	case ".ts", ".mts", ".js", ".mjs":
	default:
		return "", false
	}
	p := script
	if !filepath.IsAbs(p) {
		if dir == "" {
			dir = "."
		}
		p = filepath.Join(dir, script)
	}
	if st, err := os.Stat(p); err != nil || st.IsDir() {
		return "", false
	}
	return p, true
}

func copyEnv(in map[string]string) map[string]string {
	out := make(map[string]string, len(in)+3)
	for k, v := range in {
		out[k] = v
	}
	return out
}
