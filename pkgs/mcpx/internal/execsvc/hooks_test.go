package execsvc_test

import (
	"context"
	"encoding/json"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/execsvc"
	"github.com/dezren39/mcpx/internal/runner"
)

// readCtx is the portable top of a hook script in these tests: the hook
// context arrives on standard input as one JSON object, in every runtime
// mcpx ships (Deno's stdin readable, or the Node/Bun process stream).
const readCtx = `
const g = globalThis as any;
let raw = "";
if (g.Deno?.stdin?.readable) {
  for await (const c of g.Deno.stdin.readable) raw += new TextDecoder().decode(c);
} else if (g.process?.stdin) {
  for await (const c of g.process.stdin) raw += new TextDecoder().decode(c as any);
}
const ctx = JSON.parse(raw || "{}");
`

func hookFrames(frames []execsvc.Frame) []execsvc.Frame {
	var out []execsvc.Frame
	for _, f := range frames {
		if f.Type == execsvc.FrameHook {
			out = append(out, f)
		}
	}
	return out
}

func TestBeforeAndOnSuccessHooksRunAndTransform(t *testing.T) {
	svc := service(t, store(t))
	res, frames := run(t, svc, `
console.log(JSON.stringify({ total: 2 }));
`, execsvc.Options{Hooks: []execsvc.Hook{
		{Phase: execsvc.HookBefore, Script: `console.log(JSON.stringify({ advise: "starting" }));`},
		{Phase: execsvc.HookOnSuccess, Script: readCtx + `
console.log(JSON.stringify({ apply: { total: (ctx.result?.total ?? 0) + 1, phase: ctx.phase, run: ctx.runId } }));`},
	}})

	// The onSuccess hook replaced the result value, and only the value.
	m, ok := res.Result.(map[string]any)
	if !ok || m["total"] != float64(3) {
		t.Fatalf("the onSuccess hook should have replaced the result: %#v", res.Result)
	}
	if m["phase"] != execsvc.HookOnSuccess || m["run"] == "" || m["run"] != res.RunID {
		t.Errorf("the hook did not receive its typed context: %#v", m)
	}

	hooks := hookFrames(frames)
	if len(hooks) != 2 {
		t.Fatalf("expected 2 hook frames, got %d (%v)", len(hooks), frames)
	}
	if hooks[0].Hook.Phase != execsvc.HookBefore || hooks[0].Hook.Advice != "starting" {
		t.Errorf("before hook frame = %+v", hooks[0].Hook)
	}
	if hooks[1].Hook.Phase != execsvc.HookOnSuccess || !hooks[1].Hook.Changed {
		t.Errorf("onSuccess hook frame = %+v", hooks[1].Hook)
	}
	var orig map[string]any
	if err := json.Unmarshal(hooks[1].Hook.Original, &orig); err != nil || orig["total"] != float64(2) {
		t.Errorf("a transform must preserve the original in its frame: %s (%v)", hooks[1].Hook.Original, err)
	}

	// Frame placement: both hook frames precede the result frame they
	// surround, and the end frame is still last.
	var kinds []string
	for _, f := range frames {
		kinds = append(kinds, f.Type)
	}
	if idx(kinds, execsvc.FrameHook) > idx(kinds, execsvc.FrameResult) {
		t.Errorf("hook frames must precede the result frame: %s", strings.Join(kinds, ","))
	}
	if kinds[len(kinds)-1] != execsvc.FrameEnd {
		t.Errorf("end must be last: %s", strings.Join(kinds, ","))
	}
}

func TestOnErrorHookMayReplaceTheErrorWithAResult(t *testing.T) {
	svc := service(t, store(t))
	res, frames := run(t, svc, `
throw new Error("boom");
`, execsvc.Options{Hooks: []execsvc.Hook{
		{Phase: execsvc.HookOnError, Script: readCtx + `
console.log(JSON.stringify({ apply: { recovered: true, saw: ctx.error ?? "" } }));`},
	}})

	if res.Error != "" {
		t.Fatalf("the onError hook replaced the error, so none should remain: %q", res.Error)
	}
	m, ok := res.Result.(map[string]any)
	if !ok || m["recovered"] != true || !strings.Contains(m["saw"].(string), "boom") {
		t.Errorf("the replacement result should carry what the hook saw: %#v", res.Result)
	}
	hooks := hookFrames(frames)
	if len(hooks) != 1 || hooks[0].Hook.Phase != execsvc.HookOnError || !hooks[0].Hook.Changed {
		t.Errorf("hook frames = %+v", hooks)
	}
	for _, f := range frames {
		if f.Type == execsvc.FrameError {
			t.Errorf("no error frame may be emitted for a replaced error: %v", frames)
		}
	}
}

func TestSuffixHookRunsOnBothPathsAndCannotChangeTheOutcome(t *testing.T) {
	svc := service(t, store(t))
	suffix := execsvc.Hook{Phase: execsvc.HookSuffix, Script: `console.log(JSON.stringify({ apply: { hijacked: true }, advise: "cleaned up" }));`}

	res, frames := run(t, svc, `
console.log(JSON.stringify({ total: 2 }));
`, execsvc.Options{Hooks: []execsvc.Hook{suffix}})
	if m, ok := res.Result.(map[string]any); !ok || m["total"] != float64(2) {
		t.Errorf("a suffix hook must not change the outcome: %#v", res.Result)
	}
	hooks := hookFrames(frames)
	if len(hooks) != 1 || hooks[0].Hook.Phase != execsvc.HookSuffix || hooks[0].Hook.Advice != "cleaned up" {
		t.Fatalf("hook frames = %+v", hooks)
	}
	if len(hooks[0].Hook.Dropped) != 1 || hooks[0].Hook.Dropped[0] != "apply" {
		t.Errorf("a suffix apply must be reported dropped, not honoured or ignored: %+v", hooks[0].Hook)
	}
	// The suffix frame sits after the result frame and before the end.
	var kinds []string
	for _, f := range frames {
		kinds = append(kinds, f.Type)
	}
	if !(idx(kinds, execsvc.FrameResult) < idx(kinds, execsvc.FrameHook) && idx(kinds, execsvc.FrameHook) < len(kinds)-1) {
		t.Errorf("suffix frame placement wrong: %s", strings.Join(kinds, ","))
	}

	// And on the failing path it still runs.
	_, frames = run(t, svc, `
throw new Error("boom");
`, execsvc.Options{Hooks: []execsvc.Hook{suffix}})
	if hooks := hookFrames(frames); len(hooks) != 1 || hooks[0].Hook.Phase != execsvc.HookSuffix {
		t.Errorf("suffix hook frames on the error path = %+v", hooks)
	}
}

func TestConfigHooksLayerBeforePerCallHooks(t *testing.T) {
	svc := service(t, store(t))
	svc.Hooks = []execsvc.Hook{{Phase: execsvc.HookBefore, Name: "config", Script: `console.log(JSON.stringify({ advise: "from config" }));`}}
	_, frames := run(t, svc, `
console.log(JSON.stringify({ ok: true }));
`, execsvc.Options{Hooks: []execsvc.Hook{
		{Phase: execsvc.HookBefore, Name: "call", Script: `console.log(JSON.stringify({ advise: "from call" }));`},
	}})
	hooks := hookFrames(frames)
	if len(hooks) != 2 || hooks[0].Hook.Advice != "from config" || hooks[1].Hook.Advice != "from call" {
		t.Errorf("config hooks must run before per-call hooks: %+v", hooks)
	}
}

func TestAnUnknownHookPhaseIsRefusedBeforeAnythingRuns(t *testing.T) {
	svc := service(t, store(t))
	needRuntime(t)
	_, err := svc.RunWith(context.Background(), runner.Options{
		Source: `console.log(1);`, ClientSource: minimalClient, Permissions: "all",
	}, execsvc.Options{Hooks: []execsvc.Hook{
		{Phase: "whenever", Script: `console.log(1);`},
	}}, func(execsvc.Frame) error { return nil })
	if err == nil || !strings.Contains(err.Error(), "unknown phase") {
		t.Errorf("an unknown phase must be refused up front: %v", err)
	}
}

// Regression: successive transforms must compose. Each hook reads the
// outcome the previous hook left, so two +1 hooks on {total: 2} give 4,
// not 3 (Codex review on #2616).
func TestSuccessiveOnSuccessHooksCompose(t *testing.T) {
	svc := service(t, store(t))
	inc := readCtx + `
console.log(JSON.stringify({ apply: { total: (ctx.result?.total ?? 0) + 1 } }));`
	res, _ := run(t, svc, `
console.log(JSON.stringify({ total: 2 }));
`, execsvc.Options{Hooks: []execsvc.Hook{
		{Phase: execsvc.HookOnSuccess, Script: inc},
		{Phase: execsvc.HookOnSuccess, Script: inc},
	}})
	m, ok := res.Result.(map[string]any)
	if !ok || m["total"] != float64(4) {
		t.Fatalf("two +1 transforms on {total: 2} should compose to 4: %#v", res.Result)
	}
}

// Regression: a script that exits nonzero without writing anything has
// still failed -- it takes the onError path, and without a hook the exit
// status itself is the recorded error (Codex review on #2616).
func TestNonzeroExitWithoutOutputTakesTheErrorPath(t *testing.T) {
	svc := service(t, store(t))
	exit1 := `
const g = globalThis as any;
if (g.Deno?.exit) g.Deno.exit(1); else if (g.process?.exit) g.process.exit(1);
`
	res, _ := run(t, svc, exit1, execsvc.Options{})
	if res.Error == "" || !strings.Contains(res.Error, "code 1") {
		t.Fatalf("a bare nonzero exit should record the exit status as the error: %+v", res)
	}

	res, frames := run(t, svc, exit1, execsvc.Options{Hooks: []execsvc.Hook{
		{Phase: execsvc.HookOnError, Script: readCtx + `
console.log(JSON.stringify({ apply: { recovered: true, saw: ctx.error ?? "" } }));`},
	}})
	if res.Error != "" {
		t.Fatalf("the onError hook should have recovered the bare exit: %q", res.Error)
	}
	if m, ok := res.Result.(map[string]any); !ok || m["recovered"] != true ||
		!strings.Contains(m["saw"].(string), "code 1") {
		t.Errorf("the recovery should carry the exit-status error it saw: %#v", res.Result)
	}
	if hs := hookFrames(frames); len(hs) != 1 || hs[0].Hook.Phase != execsvc.HookOnError {
		t.Errorf("a bare nonzero exit must run onError hooks, frames = %v", frames)
	}
}

// Regression: when an onError hook replaces a failure, its frame's
// original carries the error it replaced -- for a thrown exception the
// result value is nil, so recording the result recorded nothing
// (Codex review on #2616).
func TestOnErrorFramePreservesTheOriginalError(t *testing.T) {
	svc := service(t, store(t))
	_, frames := run(t, svc, `
throw new Error("boom-original");
`, execsvc.Options{Hooks: []execsvc.Hook{
		{Phase: execsvc.HookOnError, Script: `console.log(JSON.stringify({ apply: { recovered: true } }));`},
	}})
	hs := hookFrames(frames)
	if len(hs) != 1 || !hs[0].Hook.Changed {
		t.Fatalf("expected one changed onError hook frame: %v", frames)
	}
	if !strings.Contains(string(hs[0].Hook.Original), "boom-original") {
		t.Errorf("the frame's original must preserve the replaced error: %s", hs[0].Hook.Original)
	}
}

// Regression: suffix cleanup runs when the main script never starts --
// a before hook that allocated something is owed its cleanup on the
// preflight-failure path too (Codex review on #2616).
func TestSuffixHookRunsWhenTheScriptNeverStarts(t *testing.T) {
	svc := service(t, store(t))
	res, frames := run(t, svc, `
const broken = ;
`, execsvc.Options{Hooks: []execsvc.Hook{
		{Phase: execsvc.HookSuffix, Script: `console.log(JSON.stringify({ advise: "cleaned up" }));`},
	}})
	if res.Error == "" {
		t.Fatalf("the type-broken script should have failed: %+v", res)
	}
	hs := hookFrames(frames)
	if len(hs) != 1 || hs[0].Hook.Phase != execsvc.HookSuffix || hs[0].Hook.Advice != "cleaned up" {
		t.Errorf("the suffix hook should have run on the startup-failure path: %v", frames)
	}
}
