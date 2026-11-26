package testsupport

import (
	"context"
	"encoding/json"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

// replayFakeServer is an in-process MCP server with scripted answers,
// modelled on mcpclient's own test fakes: it records nothing and
// decides each reply from the request alone, so the only source of
// nondeterminism in a recorded session would be the client itself.
type replayFakeServer struct {
	mu     sync.Mutex
	in     chan []byte
	closed bool
}

func newReplayFakeServer() *replayFakeServer {
	return &replayFakeServer{in: make(chan []byte, 64)}
}

func (s *replayFakeServer) Send(_ context.Context, msg []byte) error {
	var f map[string]any
	_ = json.Unmarshal(msg, &f)
	method, _ := f["method"].(string)
	if method == "" || f["id"] == nil {
		return nil // a notification, or a reply to a server request
	}
	params, _ := f["params"].(map[string]any)
	out := replayAnswer(method, params)
	var reply map[string]any
	if out == nil {
		reply = map[string]any{"jsonrpc": "2.0", "id": f["id"],
			"error": map[string]any{"code": -32601, "message": "no method " + method}}
	} else {
		reply = map[string]any{"jsonrpc": "2.0", "id": f["id"], "result": out}
	}
	b, _ := json.Marshal(reply)
	s.in <- b
	return nil
}

func (s *replayFakeServer) Recv() ([]byte, error) {
	msg, ok := <-s.in
	if !ok {
		return nil, context.Canceled
	}
	return msg, nil
}

func (s *replayFakeServer) Info() string { return "replay-fake" }

func (s *replayFakeServer) Close() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if !s.closed {
		s.closed = true
		close(s.in)
	}
	return nil
}

var _ mcpclient.Transport = (*replayFakeServer)(nil)

// replayAnswer is the fake server's whole behaviour: a modern
// discover, one echo tool, and an echo that returns its message
// argument as the call's text.
func replayAnswer(method string, params map[string]any) any {
	switch method {
	case "server/discover":
		return map[string]any{
			"resultType":        "complete",
			"supportedVersions": []string{"2026-07-28"},
			"_meta": map[string]any{
				"io.modelcontextprotocol/serverInfo": map[string]any{"name": "fake", "version": "1"},
			},
			"capabilities": map[string]any{"tools": map[string]any{}},
		}
	case "tools/list":
		return map[string]any{
			"resultType": "complete",
			"tools": []any{map[string]any{
				"name":        "echo",
				"description": "returns its message argument as text",
				"inputSchema": map[string]any{
					"type":       "object",
					"properties": map[string]any{"message": map[string]any{"type": "string"}},
					"required":   []string{"message"},
				},
			}},
		}
	case "tools/call":
		args, _ := params["arguments"].(map[string]any)
		text, _ := args["message"].(string)
		return map[string]any{
			"resultType": "complete",
			"content":    []any{map[string]any{"type": "text", "text": text}},
			"isError":    false,
		}
	}
	return nil
}

func dialReplayClient(t *testing.T, tr mcpclient.Transport) *mcpclient.Client {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	c, err := mcpclient.NewWithOptions(ctx, tr, mcpclient.Options{
		ClientName: "mcpx", ClientVersion: "test", Preference: mcpclient.ForceModern,
	})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = c.Close() })
	return c
}

func callText(t *testing.T, raw json.RawMessage) string {
	t.Helper()
	var res struct {
		Content []struct {
			Text string `json:"text"`
		} `json:"content"`
	}
	if err := json.Unmarshal(raw, &res); err != nil || len(res.Content) == 0 {
		t.Fatalf("call result %s does not carry text content (err %v)", raw, err)
	}
	return res.Content[0].Text
}

// runEchoSession performs the scenario under test: list the tools,
// call echo, then call echo again with the first call's own output in
// the second call's arguments — so the second request only matches a
// recording if the first response was reproduced exactly.
func runEchoSession(t *testing.T, c *mcpclient.Client) (first, second json.RawMessage) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	tools, err := c.ListTools(ctx)
	if err != nil {
		t.Fatal(err)
	}
	if len(tools) != 1 || tools[0].Name != "echo" {
		t.Fatalf("tools = %+v, want the one echo tool", tools)
	}

	first, err = c.CallTool(ctx, "echo", map[string]any{"message": "hello"})
	if err != nil {
		t.Fatal(err)
	}
	second, err = c.CallTool(ctx, "echo", map[string]any{"message": callText(t, first) + " again"})
	if err != nil {
		t.Fatal(err)
	}
	if got := callText(t, second); got != "hello again" {
		t.Fatalf("second call text = %q, want %q", got, "hello again")
	}
	return first, second
}

// recordSession runs the scenario against the fake server through a
// RecordTransport and returns the trace, round-tripped through its
// serialized form — the artifact a field capture would hand back.
func recordSession(t *testing.T) (*mcpclient.CallTrace, json.RawMessage, json.RawMessage) {
	t.Helper()
	rec := mcpclient.NewRecordTransport(newReplayFakeServer())
	c := dialReplayClient(t, rec)
	first, second := runEchoSession(t, c)
	if err := c.Close(); err != nil {
		t.Fatal(err)
	}

	data, err := rec.Trace().Marshal()
	if err != nil {
		t.Fatal(err)
	}
	trace, err := mcpclient.UnmarshalCallTrace(data)
	if err != nil {
		t.Fatal(err)
	}
	return trace, first, second
}

func TestRecordReplayReproducesSession(t *testing.T) {
	trace, wantFirst, wantSecond := recordSession(t)

	rt := NewReplayTransport(trace)
	c := dialReplayClient(t, rt)
	gotFirst, gotSecond := runEchoSession(t, c)
	if err := c.Close(); err != nil {
		t.Fatal(err)
	}

	if string(gotFirst) != string(wantFirst) || string(gotSecond) != string(wantSecond) {
		t.Fatalf("replayed results differ:\n got %s / %s\nwant %s / %s",
			gotFirst, gotSecond, wantFirst, wantSecond)
	}
	if err := rt.Finish(); err != nil {
		t.Fatal(err)
	}
}

func TestReplayDivergesOnDifferentCall(t *testing.T) {
	trace, _, _ := recordSession(t)

	rt := NewReplayTransport(trace)
	c := dialReplayClient(t, rt)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	if _, err := c.ListTools(ctx); err != nil {
		t.Fatal(err)
	}
	_, err := c.CallTool(ctx, "echo", map[string]any{"message": "not what was recorded"})
	if err == nil {
		t.Fatal("CallTool with unrecorded arguments succeeded under replay")
	}
	if !strings.Contains(err.Error(), "replay divergence") {
		t.Fatalf("err = %v, want a replay divergence naming the frame", err)
	}
	_ = c.Close()
	if ferr := rt.Finish(); ferr == nil {
		t.Fatal("Finish after a divergence reported success")
	}
}

func TestReplayFinishReportsUnplayedFrames(t *testing.T) {
	trace, _, _ := recordSession(t)

	rt := NewReplayTransport(trace)
	c := dialReplayClient(t, rt)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	if _, err := c.ListTools(ctx); err != nil {
		t.Fatal(err)
	}
	_ = c.Close()

	err := rt.Finish()
	if err == nil {
		t.Fatal("Finish accepted a replay that never made the recorded calls")
	}
	if !strings.Contains(err.Error(), "never played") {
		t.Fatalf("Finish err = %v, want it to name the unplayed frames", err)
	}
}

func TestReplayTruncatedTraceFailsTheCall(t *testing.T) {
	trace, _, _ := recordSession(t)
	// Drop the final frame — the second call's response. The replayed
	// client must not invent an answer: the call fails the way a call
	// to a server that never replies fails, on its context deadline.
	trace.Frames = trace.Frames[:len(trace.Frames)-1]

	rt := NewReplayTransport(trace)
	c := dialReplayClient(t, rt)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	if _, err := c.ListTools(ctx); err != nil {
		t.Fatal(err)
	}
	first, err := c.CallTool(ctx, "echo", map[string]any{"message": "hello"})
	if err != nil {
		t.Fatal(err)
	}
	short, shortCancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer shortCancel()
	if _, err := c.CallTool(short, "echo", map[string]any{"message": callText(t, first) + " again"}); err == nil {
		t.Fatal("CallTool succeeded under a trace missing its response")
	}
	_ = c.Close()
}
