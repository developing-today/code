package execsvc_test

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/artifacts"
	"github.com/developing-today/code/pkgs/mcpx/internal/codegen"
	"github.com/developing-today/code/pkgs/mcpx/internal/execsvc"
	"github.com/developing-today/code/pkgs/mcpx/internal/runner"
)

func store(t *testing.T) *artifacts.Store {
	t.Helper()
	s, err := artifacts.Open(artifacts.Options{Dir: t.TempDir(), TTL: time.Hour})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = s.Close() })
	return s
}

func service(t *testing.T, st *artifacts.Store) *execsvc.Service {
	t.Helper()
	lim := execsvc.DefaultLimits()
	// No runtime is started in these tests; a short bound means a mistake
	// fails rather than hangs.
	lim.Timeout = 5 * time.Second
	return &execsvc.Service{Store: st, Limits: lim, Local: true}
}

func TestScriptCheckPoliciesAreCarriedInExecOptions(t *testing.T) {
	want := execsvc.Options{TypeCheckOnError: "warn", TypeCheckOnUnsupported: "switch"}
	data, err := json.Marshal(want)
	if err != nil {
		t.Fatal(err)
	}
	var got execsvc.Options
	if err := json.Unmarshal(data, &got); err != nil {
		t.Fatal(err)
	}
	if got.TypeCheckOnError != want.TypeCheckOnError || got.TypeCheckOnUnsupported != want.TypeCheckOnUnsupported {
		t.Fatalf("round trip = %+v, want %+v", got, want)
	}
	for _, field := range []string{`"checkOnError":"warn"`, `"checkOnUnsupported":"switch"`} {
		if !strings.Contains(string(data), field) {
			t.Errorf("wire options %s do not include %s", data, field)
		}
	}
}

// TestCheckWarnDiagnosticReachesSuccessResult is the Codex finding on
// PR #6111: with checkOnError=warn a non-fatal check diagnostic must
// reach a remote caller's result and frames even when the script runs
// successfully. The diagnostic went only to the stderr tail buffer,
// which this service exposes only on a non-zero exit, so a warn-policy
// run was indistinguishable from `run`. A Node runtime with
// typecheck=types produces the diagnostic deterministically (Node
// strips types rather than checking them), so the delivery path is
// exercised without a checker fixture.
func TestCheckWarnDiagnosticReachesSuccessResult(t *testing.T) {
	if _, err := runner.Resolve("node", "", runner.Setup{}); err != nil {
		t.Skip("no node runtime:", err)
	}
	svc := service(t, store(t))
	var frames []execsvc.Frame
	res, err := svc.RunWith(context.Background(), runner.Options{
		Source:       `console.log("ok");`,
		ClientSource: minimalClient,
		Permissions:  "all",
		// The policies travel on the runner options RunWith receives
		// (the service's Run builds them from execsvc.Options); a
		// direct RunWith caller supplies them here.
		Runtime:                "node",
		TypeCheck:              "types",
		TypeCheckOnError:       "warn",
		TypeCheckOnUnsupported: "warn",
	}, execsvc.Options{},
		func(f execsvc.Frame) error {
			frames = append(frames, f)
			return nil
		})
	if err != nil {
		t.Fatalf("run: %v (%+v)", err, res)
	}
	if res.Error != "" || res.ExitCode != 0 {
		t.Fatalf("a warn policy must not fail the run: %+v", res)
	}
	const wantMsg = "only available under Deno"
	foundLog := false
	for _, l := range res.Logs {
		if l.Level == "warn" && strings.Contains(l.Message, wantMsg) {
			foundLog = true
		}
	}
	if !foundLog {
		t.Fatalf("the result's logs do not carry the check warning: %+v", res.Logs)
	}
	foundFrame := false
	for _, f := range frames {
		if f.Type == execsvc.FrameLog && f.Level == "warn" && strings.Contains(f.Message, wantMsg) {
			foundFrame = true
		}
	}
	if !foundFrame {
		t.Fatalf("the streamed frames do not carry the check warning: %+v", frames)
	}
}

// The runtime is a real one. Skipping rather than failing keeps the suite
// honest on a machine that has none: these tests are about the service, and
// a missing deno is not a bug in it.
func needRuntime(t *testing.T) {
	t.Helper()
	if _, err := runner.Resolve("auto", "", runner.Setup{}); err != nil {
		t.Skip("no JavaScript runtime:", err)
	}
}

func run(t *testing.T, svc *execsvc.Service, source string, opts execsvc.Options) (*execsvc.Result, []execsvc.Frame) {
	t.Helper()
	needRuntime(t)
	var frames []execsvc.Frame
	res, err := svc.RunWith(context.Background(), runner.Options{
		Source:       source,
		ClientSource: minimalClient,
		Permissions:  "all",
	}, opts, func(f execsvc.Frame) error {
		frames = append(frames, f)
		return nil
	})
	if err != nil {
		t.Fatalf("run: %v (%+v)", err, res)
	}
	return res, frames
}

// minimalClient is enough of the generated module for the launcher's import
// to resolve. The real one is generated from live servers, which these tests
// deliberately do not need.
const minimalClient = `
export const log = Object.assign(() => {}, {
  debug(){}, info(m: string){ write("log", { level: "info", msg: m }); },
  warn(){}, error(){}, with(){ return log; }, enabled(){ return true; },
});
function write(kind: string, payload: Record<string, unknown>) {
  const line = "\u001emcpx\u001e" + JSON.stringify({ kind, ts: new Date().toISOString(), ...payload });
  const enc = new TextEncoder().encode(line + "\n");
  (globalThis as any).Deno?.stderr?.writeSync?.(enc) ??
    (globalThis as any).process?.stderr?.write?.(line + "\n");
}
export function emit(value: unknown) { write("result", { value }); }
export const emitResult = emit;
export const tools = {};
export function installGlobals() {
  const g = globalThis as any;
  g.log ??= log; g.emit ??= emit;
}
export function captureConsole() {}
export function releaseConsole() {}
export function call() {}
export function readResource() {}
export function artifact() {}
export class ToolError extends Error {}
export default tools;
`

func TestFramesArriveInOrderAndEndLast(t *testing.T) {
	svc := service(t, store(t))
	_, frames := run(t, svc, `
import { log, emit, installGlobals } from "./mcpx-client.ts";
installGlobals();
log.info("working");
emit({ step: 1 });
console.log(JSON.stringify({ done: true }));
`, execsvc.Options{})

	var kinds []string
	for _, f := range frames {
		kinds = append(kinds, f.Type)
	}
	joined := strings.Join(kinds, ",")
	if !strings.HasPrefix(joined, "start,") {
		t.Errorf("a run must announce itself first: %s", joined)
	}
	if kinds[len(kinds)-1] != execsvc.FrameEnd {
		t.Errorf("end must be last when nothing is streamed: %s", joined)
	}
	for _, want := range []string{execsvc.FrameLog, execsvc.FrameEmit, execsvc.FrameStdout, execsvc.FrameResult} {
		if !strings.Contains(joined, want) {
			t.Errorf("missing %s frame: %s", want, joined)
		}
	}
	if idx(kinds, execsvc.FrameResult) > idx(kinds, execsvc.FrameEnd) {
		t.Errorf("the result must come before the end: %s", joined)
	}
}

func idx(s []string, want string) int {
	for i, v := range s {
		if v == want {
			return i
		}
	}
	return len(s)
}

func TestStructuredResultCarriesEverything(t *testing.T) {
	svc := service(t, store(t))
	res, _ := run(t, svc, `
import { log, emit, installGlobals } from "./mcpx-client.ts";
installGlobals();
log.info("a note");
emit({ n: 1 });
emit({ n: 2 });
console.log(JSON.stringify({ total: 2 }));
`, execsvc.Options{})

	if len(res.Emits) != 2 {
		t.Errorf("emits = %d", len(res.Emits))
	}
	if len(res.Logs) != 1 || res.Logs[0].Message != "a note" {
		t.Errorf("logs = %+v", res.Logs)
	}
	if !strings.Contains(res.Stdout, "total") {
		t.Errorf("stdout = %q", res.Stdout)
	}
	m, ok := res.Result.(map[string]any)
	if !ok || m["total"] != float64(2) {
		t.Errorf("JSON on stdout should be offered parsed, got %#v", res.Result)
	}
	if res.ExitCode != 0 {
		t.Errorf("exit = %d, error = %q", res.ExitCode, res.Error)
	}
	if res.RunID == "" {
		t.Error("a run should be identifiable")
	}
}

// A consumer that leaves must stop the script. Otherwise a cancelled stream
// leaves work running on the daemon that nobody will ever read.
func TestASinkThatFailsStopsTheRun(t *testing.T) {
	needRuntime(t)
	svc := service(t, store(t))
	// The failure is deferred past the start frame so that what is being
	// tested is a consumer leaving mid-run, not one that was never there.
	frames := 0
	started := time.Now()
	_, err := svc.RunWith(context.Background(), runner.Options{
		Source: `
import { log, installGlobals } from "./mcpx-client.ts";
installGlobals();
log.info("running");
await new Promise((r) => setTimeout(r, 60000));
`,
		ClientSource: minimalClient,
		Permissions:  "all",
	}, execsvc.Options{}, func(f execsvc.Frame) error {
		frames++
		if f.Type == execsvc.FrameLog {
			return errClosed
		}
		return nil
	})
	if err == nil {
		t.Fatal("the sink's error should surface")
	}
	if frames < 2 {
		t.Fatalf("the run should have got past start, saw %d frames", frames)
	}
	// The script asked to sleep a minute. Anything near that means the
	// cancelled context did not reach the process group.
	if d := time.Since(started); d > 20*time.Second {
		t.Errorf("the script outlived its consumer by %s", d)
	}
}

var errClosed = &closedErr{}

type closedErr struct{}

func (*closedErr) Error() string { return "consumer went away" }

// An artifact in the index for this run is found even when the script never
// announced it, and delivered under the caller's terms.
func TestArtifactsAreReconciledAndDeliveredIntoADirectory(t *testing.T) {
	needRuntime(t)
	st := store(t)
	svc := service(t, st)
	runID := execsvc.NewRunID()
	if _, err := st.Put(strings.NewReader("BODY"), artifacts.PutOptions{
		Name: "shot.png", Run: runID,
	}); err != nil {
		t.Fatal(err)
	}
	out := filepath.Join(t.TempDir(), "collected")

	var frames []execsvc.Frame
	res, err := svc.RunWith(context.Background(), runner.Options{
		Source:       "console.log(\"ok\");",
		ClientSource: minimalClient,
		Permissions:  "all",
		Env:          map[string]string{"MCPX_RUN": runID},
	}, execsvc.Options{
		Capabilities: []string{execsvc.CapabilityArtifacts},
		Artifacts:    &execsvc.ArtifactOptions{Dir: out},
	}, func(f execsvc.Frame) error { frames = append(frames, f); return nil })
	if err != nil {
		t.Fatal(err)
	}
	if len(res.Artifacts) != 1 {
		t.Fatalf("the index should have been consulted: %+v", res.Artifacts)
	}
	got := res.Artifacts[0]
	if got.Path == "" {
		t.Fatal("a named directory should have received the file")
	}
	body, rerr := os.ReadFile(got.Path)
	if rerr != nil {
		t.Fatal(rerr)
	}
	if string(body) != "BODY" {
		t.Errorf("body = %q", body)
	}
	if filepath.Base(got.Path) != "shot.png" {
		t.Errorf("name = %s", filepath.Base(got.Path))
	}
	// Placed, not encoded: a caller that can read the directory has no use
	// for a base64 copy of what is already on its disk.
	if got.Data != "" {
		t.Error("a placed artifact should not also be inlined")
	}
}

func TestInlineDeliveryBoundsWhatItEncodes(t *testing.T) {
	needRuntime(t)
	st := store(t)
	svc := service(t, st)
	runID := execsvc.NewRunID()
	if _, err := st.Put(strings.NewReader("0123456789"), artifacts.PutOptions{
		Name: "big.bin", Run: runID,
	}); err != nil {
		t.Fatal(err)
	}
	res, err := svc.RunWith(context.Background(), runner.Options{
		Source:       "console.log(\"ok\");",
		ClientSource: minimalClient,
		Permissions:  "all",
		Env:          map[string]string{"MCPX_RUN": runID},
	}, execsvc.Options{
		Capabilities: []string{execsvc.CapabilityArtifacts},
		Artifacts:    &execsvc.ArtifactOptions{Delivery: execsvc.DeliveryInline, MaxBytes: 4},
	}, nil)
	if err != nil {
		t.Fatal(err)
	}
	if res.Artifacts[0].Data != "" {
		t.Error("an artifact over the caller's ceiling should stay a reference")
	}
	if res.Artifacts[0].URI == "" {
		t.Error("a reference still has to say where to find it")
	}
}

func TestStreamedBodiesComeAfterTheEndFrame(t *testing.T) {
	needRuntime(t)
	st := store(t)
	svc := service(t, st)
	runID := execsvc.NewRunID()
	if _, err := st.Put(strings.NewReader("streamed body"), artifacts.PutOptions{
		Name: "s.txt", Run: runID,
	}); err != nil {
		t.Fatal(err)
	}
	var kinds []string
	var data strings.Builder
	_, err := svc.RunWith(context.Background(), runner.Options{
		Source:       "console.log(\"ok\");",
		ClientSource: minimalClient,
		Permissions:  "all",
		Env:          map[string]string{"MCPX_RUN": runID},
	}, execsvc.Options{
		Capabilities: []string{execsvc.CapabilityArtifacts},
		Artifacts:    &execsvc.ArtifactOptions{Delivery: execsvc.DeliveryStream},
	}, func(f execsvc.Frame) error {
		kinds = append(kinds, f.Type)
		if f.Type == execsvc.FrameArtifactData {
			raw, derr := base64.StdEncoding.DecodeString(f.Data)
			if derr != nil {
				return derr
			}
			data.Write(raw)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if data.String() != "streamed body" {
		t.Errorf("reassembled = %q", data.String())
	}
	end := idx(kinds, execsvc.FrameEnd)
	chunk := idx(kinds, execsvc.FrameArtifactData)
	if chunk < end {
		t.Errorf("bodies must come after end so a consumer can cancel first: %v", kinds)
	}
	if kinds[len(kinds)-1] != execsvc.FrameArtifactEnd {
		t.Errorf("each body should be closed off: %v", kinds)
	}
}

// The point of interception: a megabyte of base64 in a result becomes a
// reference, and only when the caller said it can receive one.
func TestInlineImagesBecomeArtifactReferences(t *testing.T) {
	needRuntime(t)
	st := store(t)
	svc := service(t, st)
	png := base64.StdEncoding.EncodeToString([]byte("\x89PNG-bytes"))
	source := `
import { emit, installGlobals } from "./mcpx-client.ts";
installGlobals();
console.log(JSON.stringify({ content: [{ type: "image", data: "` + png + `", mimeType: "image/png" }] }));
`
	res, _ := run(t, svc, source, execsvc.Options{
		Capabilities: []string{execsvc.CapabilityArtifacts},
	})
	blob, _ := json.Marshal(res.Result)
	if strings.Contains(string(blob), png) {
		t.Errorf("the base64 survived into the result: %s", blob)
	}
	if !strings.Contains(string(blob), "resource_link") {
		t.Errorf("want a resource_link, got %s", blob)
	}
	if len(res.Artifacts) != 1 {
		t.Fatalf("the bytes should have been stored: %+v", res.Artifacts)
	}
	if res.Artifacts[0].Mime != "image/png" {
		t.Errorf("mime = %q", res.Artifacts[0].Mime)
	}
	body, _, err := st.Bytes(res.Artifacts[0].ID)
	if err != nil {
		t.Fatal(err)
	}
	if string(body) != "\x89PNG-bytes" {
		t.Errorf("stored body = %q", body)
	}
}

func TestInlineImagesAreLeftAloneWhenTheCallerCannotReceiveArtifacts(t *testing.T) {
	needRuntime(t)
	svc := service(t, store(t))
	png := base64.StdEncoding.EncodeToString([]byte("\x89PNG"))
	res, _ := run(t, svc, `
import { installGlobals } from "./mcpx-client.ts";
installGlobals();
console.log(JSON.stringify({ content: [{ type: "image", data: "`+png+`", mimeType: "image/png" }] }));
`, execsvc.Options{})
	blob, _ := json.Marshal(res.Result)
	if !strings.Contains(string(blob), png) {
		t.Errorf("a caller that declared nothing should get what the tool sent: %s", blob)
	}
}

func TestWantsArtifactsReadsBothWaysOfAskingForThem(t *testing.T) {
	if (execsvc.Options{}).WantsArtifacts() {
		t.Error("silence is not a declaration")
	}
	if !(execsvc.Options{Capabilities: []string{execsvc.CapabilityArtifacts}}).WantsArtifacts() {
		t.Error("the declared capability should count")
	}
	if !(execsvc.Options{Artifacts: &execsvc.ArtifactOptions{Dir: "/tmp"}}).WantsArtifacts() {
		t.Error("naming a directory is declaring the capability")
	}
}

func TestAFailedScriptExplainsItself(t *testing.T) {
	// A script that runs and exits non-zero is not an error to the runner,
	// so nothing filled in Error and the script's stderr was discarded: the
	// caller got exitCode 1 and nothing else. Over /v1 that is the whole
	// diagnostic an agent would ever see.
	needRuntime(t)
	svc := service(t, store(t))
	for _, src := range []string{
		`throw new Error("boom");`,
		`return 1;`, // illegal at the top level of a module
		`nonexistent_fn();`,
	} {
		res, _ := svc.RunWith(context.Background(), runner.Options{
			Source: src, ClientSource: minimalClient, Permissions: "all",
		}, execsvc.Options{Output: execsvc.OutputStructured}, func(execsvc.Frame) error { return nil })
		if res == nil || res.ExitCode == 0 {
			t.Errorf("%s: expected a failure", src)
			continue
		}
		if strings.TrimSpace(res.Error) == "" {
			t.Errorf("%s: exit %d with no error to show for it", src, res.ExitCode)
		}
	}
}

// The sink is called from two goroutines -- the one copying the script's
// stdout and the one parsing records off its stderr. An HTTP stream writes
// each frame straight to the response, so two calls at once interleave
// bytes and corrupt the chunked encoding ("chunked line ends with bare LF"),
// which is how TestAStreamConsumerCanCancelAfterTheResult lost its end frame
// (#270). One sink call at a time is the contract.
func TestTheSinkIsNeverCalledConcurrently(t *testing.T) {
	needRuntime(t)
	svc := service(t, store(t))
	var inFlight, overlaps, emits, stdouts atomic.Int32
	_, err := svc.RunWith(context.Background(), runner.Options{
		Source: `
import { emit } from "./mcpx-client.ts";
for (let i = 0; i < 200; i++) { console.log("out " + i); emit({ i }); }
`,
		ClientSource: minimalClient,
		Permissions:  "all",
	}, execsvc.Options{}, func(f execsvc.Frame) error {
		switch f.Type {
		case execsvc.FrameEmit:
			emits.Add(1)
		case execsvc.FrameStdout:
			stdouts.Add(1)
		}
		if inFlight.Add(1) > 1 {
			overlaps.Add(1)
		}
		// Widens the window, so an unserialised sink is caught every run
		// rather than occasionally.
		time.Sleep(200 * time.Microsecond)
		inFlight.Add(-1)
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	if emits.Load() < 200 || stdouts.Load() < 2 {
		t.Fatalf("premise: frames must come from both streams; %d emit, %d stdout",
			emits.Load(), stdouts.Load())
	}
	if n := overlaps.Load(); n > 0 {
		t.Fatalf("the sink was entered concurrently %d times", n)
	}
}

// A shebang file runs through the shared service entry point -- the one
// /v1/exec and the CLI both pass through -- under its own interpreter,
// with output and exit code propagating (#2167). No JavaScript runtime
// is involved, so this test never skips for one.
func TestRunWithShebangFile(t *testing.T) {
	svc := service(t, store(t))
	dir := t.TempDir()
	path := filepath.Join(dir, "greet.sh")
	if err := os.WriteFile(path, []byte("#!/bin/sh\necho execsvc-shebang-$MCPX_RUNTIME\nexit 5\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	var frames []execsvc.Frame
	res, err := svc.RunWith(context.Background(), runner.Options{
		File: path, Permissions: "all",
	}, execsvc.Options{}, func(f execsvc.Frame) error {
		frames = append(frames, f)
		return nil
	})
	if err != nil {
		t.Fatalf("RunWith: %v", err)
	}
	if res.ExitCode != 5 {
		t.Errorf("ExitCode = %d, want 5", res.ExitCode)
	}
	if !strings.Contains(res.Stdout, "execsvc-shebang-sh") {
		t.Errorf("Stdout = %q, want the script's output with MCPX_RUNTIME=sh", res.Stdout)
	}
	if res.Runtime != "sh" {
		t.Errorf("Runtime = %q, want sh", res.Runtime)
	}
}

// Oracle: session ownership (Codex P1 on PR #8317). A run that borrows
// its session from a still-running caller must not release the
// session's instances when it ends; a run that owns its session
// releases exactly once, with that session.
func TestBorrowedSessionIsNotReleased(t *testing.T) {
	for _, tc := range []struct {
		name    string
		borrow  bool
		wantRel int
	}{
		{"borrowed", true, 0},
		{"owned", false, 1},
	} {
		t.Run(tc.name, func(t *testing.T) {
			svc := service(t, store(t))
			var released []string
			svc.Release = func(_ context.Context, session string) error {
				released = append(released, session)
				return nil
			}
			run(t, svc, `console.log(JSON.stringify({ ok: true }));`, execsvc.Options{
				Session: "outer-session", BorrowSession: tc.borrow,
			})
			if len(released) != tc.wantRel {
				t.Fatalf("release calls = %v, want %d", released, tc.wantRel)
			}
			if tc.wantRel == 1 && released[0] != "outer-session" {
				t.Errorf("released session = %q, want outer-session", released[0])
			}
		})
	}
}

// CODE-30 (#1937): a failure carries a typed kind beside its message,
// so a caller can branch without parsing text -- and success carries
// none.
func TestFailureKindsAreTyped(t *testing.T) {
	needRuntime(t)
	svc := service(t, store(t))
	run := func(ropts runner.Options) *execsvc.Result {
		res, _ := svc.RunWith(context.Background(), ropts, execsvc.Options{Output: execsvc.OutputStructured},
			func(execsvc.Frame) error { return nil })
		if res == nil {
			t.Fatal("no result")
		}
		return res
	}

	// The script started and threw: an execution failure, its own
	// message preserved.
	res := run(runner.Options{
		Source: `throw new Error("boom");`, ClientSource: minimalClient, Permissions: "all",
	})
	if res.ErrorKind != execsvc.FailureExecution || !strings.Contains(res.Error, "boom") {
		t.Errorf("thrown error: kind = %q, error = %q", res.ErrorKind, res.Error)
	}

	// The script outlived its bound: a timeout, distinct from the
	// failure it also reports.
	res = run(runner.Options{
		Source:       `await new Promise((r) => setTimeout(r, 60000));`,
		ClientSource: minimalClient, Permissions: "all", Timeout: time.Second,
	})
	if !res.TimedOut || res.ErrorKind != execsvc.FailureTimeout {
		t.Errorf("timeout: timedOut = %v, kind = %q, error = %q", res.TimedOut, res.ErrorKind, res.Error)
	}

	// No such runtime: the script never started.
	res = run(runner.Options{
		Source: `console.log("ok");`, ClientSource: minimalClient, Permissions: "all",
		Runtime: "no-such-runtime-9316",
	})
	if res.ErrorKind != execsvc.FailureNotStarted || res.Error == "" {
		t.Errorf("never started: kind = %q, error = %q", res.ErrorKind, res.Error)
	}

	// A clean run has no kind.
	res = run(runner.Options{
		Source: `console.log("ok");`, ClientSource: minimalClient, Permissions: "all",
	})
	if res.Error != "" || res.ErrorKind != "" {
		t.Errorf("clean run: kind = %q, error = %q", res.ErrorKind, res.Error)
	}
}

// CODE-36 (#1937): in-script search finds saved scripts (recipes)
// beside tools, each match carrying its kind, and pages the combined
// list in one stable order.
func TestSearchFindsRecipesAlongsideTools(t *testing.T) {
	needRuntime(t)
	nss := []codegen.Namespace{{
		Name: "media", Server: "media-server", Description: "media tools",
		Tools: []codegen.Tool{{
			Name:        "capture_screenshot",
			Description: "Capture a screenshot of the page",
			InputSchema: json.RawMessage(`{"type":"object","properties":{"url":{"type":"string"}},"required":["url"]}`),
		}},
	}}
	module := codegen.ModuleWithRecipes(nss, "http://127.0.0.1:1", "", []codegen.RecipeMeta{
		{Name: "screenshot-and-diff", Description: "Screenshot a page and diff it against a baseline", Params: []string{"url", "baseline"}, Required: []string{"url"}},
	})
	svc := service(t, store(t))
	res, err := svc.RunWith(context.Background(), runner.Options{
		Source:       `const hits = search("screenshot"); console.log(JSON.stringify(hits.map((h: any) => h.kind + ":" + (h.kind === "tool" ? h.tool : h.name) + ":" + h.params.join(","))));`,
		ClientSource: module, Permissions: "all",
	}, execsvc.Options{Output: execsvc.OutputStructured}, func(execsvc.Frame) error { return nil })
	if err != nil {
		t.Fatal(err)
	}
	if res == nil {
		t.Fatal("no result")
	}
	for _, want := range []string{"recipe:screenshot-and-diff:url,baseline", "tool:capture_screenshot:url"} {
		if !strings.Contains(res.Stdout, want) {
			t.Errorf("search output %q missing %q", res.Stdout, want)
		}
	}
	if res.Error != "" {
		t.Errorf("run error: %q", res.Error)
	}
}

// CODE-30 (#1937; Codex review, PR #9205): a bound that expires during
// runner preflight -- here a frozen install whose npm never finishes --
// is a timeout even though the script never started.
func TestFailureKindPreflightTimeout(t *testing.T) {
	needRuntime(t)
	binDir := t.TempDir()
	stub := "#!/bin/sh\nexec sleep 30\n"
	if err := os.WriteFile(filepath.Join(binDir, "npm"), []byte(stub), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", binDir+string(os.PathListSeparator)+os.Getenv("PATH"))
	svc := service(t, store(t))
	res, _ := svc.RunWith(context.Background(), runner.Options{
		Source: "export default () => 1", Runtime: "node",
		ClientSource: minimalClient, Permissions: "all", Timeout: time.Second,
		Deps: &runner.Deps{Lockfile: []byte("locked"), LockfileName: "package-lock.json"},
	}, execsvc.Options{Output: execsvc.OutputStructured},
		func(execsvc.Frame) error { return nil })
	if res == nil {
		t.Fatal("no result")
	}
	if !res.TimedOut || res.ErrorKind != execsvc.FailureTimeout {
		t.Errorf("preflight timeout: timedOut = %v, kind = %q, error = %q", res.TimedOut, res.ErrorKind, res.Error)
	}
}

// CODE-19 (#1937): a snippet's final top-level expression is its
// result without print/emit; stdout and emits stay separately
// observable, and the expression outranks the stdout JSON parse.
func TestFinalExpressionIsTheResult(t *testing.T) {
	needRuntime(t)
	module := codegen.Module(nil, "http://127.0.0.1:1", "")
	svc := service(t, store(t))
	run := func(src string) *execsvc.Result {
		res, err := svc.RunWith(context.Background(), runner.Options{
			Source: src, ClientSource: module, Permissions: "all",
		}, execsvc.Options{Output: execsvc.OutputStructured}, func(execsvc.Frame) error { return nil })
		if err != nil {
			t.Fatal(err)
		}
		if res == nil {
			t.Fatal("no result")
		}
		if res.Error != "" {
			t.Fatalf("run of %q failed: %s", src, res.Error)
		}
		return res
	}

	// A bare final expression, nothing printed at all.
	res := run(`1 + 2`)
	if res.Result != float64(3) {
		t.Errorf("bare expression: result = %#v, want 3", res.Result)
	}

	// Prints and emits along the way stay observable under their
	// own names; the final expression is the result even though
	// stdout carried JSON of its own.
	res = run(`console.log(JSON.stringify({ printed: true })); emit("mid"); const out = { answer: 42 }; out`)
	if got, ok := res.Result.(map[string]any); !ok || got["answer"] != float64(42) {
		t.Errorf("final expression: result = %#v, want the answer object", res.Result)
	}
	if !strings.Contains(res.Stdout, "printed") {
		t.Errorf("stdout lost the printed JSON: %q", res.Stdout)
	}
	if len(res.Emits) != 1 || string(res.Emits[0]) != `"mid"` {
		t.Errorf("emits = %v, want the single mid emit kept apart", res.Emits)
	}

	// A declaration tail is not an expression: the stdout rule
	// still decides.
	res = run(`const x = JSON.stringify({ via: "stdout" }); console.log(x);`)
	if got, ok := res.Result.(map[string]any); !ok || got["via"] != "stdout" {
		t.Errorf("declaration tail: result = %#v, want the stdout object", res.Result)
	}
}
