package mcpclient_test

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

func sampleTrace() *mcpclient.CallTrace {
	return &mcpclient.CallTrace{Frames: []mcpclient.CallFrame{
		{Dir: mcpclient.TraceSend, Msg: json.RawMessage(`{"jsonrpc":"2.0","id":1,"method":"tools/list"}`)},
		{Dir: mcpclient.TraceRecv, Msg: json.RawMessage(`{"jsonrpc":"2.0","id":1,"result":{"tools":[]}}`)},
		{Dir: mcpclient.TraceSend, Msg: json.RawMessage(`{"jsonrpc":"2.0","method":"notifications/initialized"}`)},
	}}
}

func TestCallTraceRoundTrip(t *testing.T) {
	data, err := sampleTrace().Marshal()
	if err != nil {
		t.Fatal(err)
	}
	back, err := mcpclient.UnmarshalCallTrace(data)
	if err != nil {
		t.Fatal(err)
	}
	if back.Format != mcpclient.CallTraceFormat {
		t.Fatalf("format = %q, want %q", back.Format, mcpclient.CallTraceFormat)
	}
	want := sampleTrace()
	if len(back.Frames) != len(want.Frames) {
		t.Fatalf("frames = %d, want %d", len(back.Frames), len(want.Frames))
	}
	for i, f := range back.Frames {
		if f.Dir != want.Frames[i].Dir || string(f.Msg) != string(want.Frames[i].Msg) {
			t.Fatalf("frame %d = %s %s, want %s %s", i, f.Dir, f.Msg, want.Frames[i].Dir, want.Frames[i].Msg)
		}
	}
}

func TestUnmarshalCallTraceRejects(t *testing.T) {
	cases := map[string]string{
		"unknown version": `{"format":"mcpx-call-trace/v99","frames":[{"dir":"send","msg":{}}]}`,
		"missing version": `{"frames":[{"dir":"send","msg":{}}]}`,
		"no frames":       `{"format":"mcpx-call-trace/v1","frames":[]}`,
		"bad direction":   `{"format":"mcpx-call-trace/v1","frames":[{"dir":"sideways","msg":{}}]}`,
		"frame not json":  `{"format":"mcpx-call-trace/v1","frames":[{"dir":"send","msg":}]}`,
		"not json at all": `hello`,
	}
	for name, data := range cases {
		if _, err := mcpclient.UnmarshalCallTrace([]byte(data)); err == nil {
			t.Errorf("%s: UnmarshalCallTrace accepted %s", name, data)
		}
	}
}

// stubTransport is a Transport with scripted outcomes, for recorder
// tests that must control exactly which frames the inner transport
// accepts.
type stubTransport struct {
	sendErr error
	recvQ   [][]byte
	recvErr error
}

func (s *stubTransport) Send(context.Context, []byte) error { return s.sendErr }
func (s *stubTransport) Recv() ([]byte, error) {
	if len(s.recvQ) == 0 {
		return nil, s.recvErr
	}
	msg := s.recvQ[0]
	s.recvQ = s.recvQ[1:]
	return msg, nil
}
func (s *stubTransport) Close() error   { return nil }
func (s *stubTransport) Info() string   { return "stub" }
var _ mcpclient.Transport = (*stubTransport)(nil)

func TestRecordTransportRecordsAcceptedFramesInOrder(t *testing.T) {
	inner := &stubTransport{recvQ: [][]byte{
		[]byte(`{"jsonrpc":"2.0","id":1,"result":{}}`),
		[]byte(`{"jsonrpc":"2.0","method":"notifications/message"}`),
	}}
	rec := mcpclient.NewRecordTransport(inner)
	ctx := context.Background()

	if err := rec.Send(ctx, []byte(`{"jsonrpc":"2.0","id":1,"method":"tools/list"}`)); err != nil {
		t.Fatal(err)
	}
	if _, err := rec.Recv(); err != nil {
		t.Fatal(err)
	}
	if _, err := rec.Recv(); err != nil {
		t.Fatal(err)
	}
	if err := rec.Send(ctx, []byte(`{"jsonrpc":"2.0","method":"notifications/initialized"}`)); err != nil {
		t.Fatal(err)
	}

	trace := rec.Trace()
	if trace.Format != mcpclient.CallTraceFormat {
		t.Fatalf("format = %q", trace.Format)
	}
	wantDirs := []mcpclient.TraceDir{mcpclient.TraceSend, mcpclient.TraceRecv, mcpclient.TraceRecv, mcpclient.TraceSend}
	if len(trace.Frames) != len(wantDirs) {
		t.Fatalf("frames = %d, want %d", len(trace.Frames), len(wantDirs))
	}
	for i, dir := range wantDirs {
		if trace.Frames[i].Dir != dir {
			t.Fatalf("frame %d dir = %s, want %s", i, trace.Frames[i].Dir, dir)
		}
	}
	if !strings.Contains(rec.Info(), "stub") {
		t.Fatalf("Info = %q, want it to name the wrapped transport", rec.Info())
	}

	// The returned trace is a copy: clearing it must not reach back
	// into the recorder.
	trace.Frames[0].Msg = json.RawMessage(`{}`)
	if got := rec.Trace().Frames[0].Msg; strings.Contains(string(got), `"method":"tools/list"`) == false {
		t.Fatalf("recorder frame changed through the returned trace: %s", got)
	}
}

func TestRecordTransportSkipsFailedFrames(t *testing.T) {
	boom := errors.New("boom")
	ctx := context.Background()

	rec := mcpclient.NewRecordTransport(&stubTransport{sendErr: boom, recvErr: boom})
	if err := rec.Send(ctx, []byte(`{"jsonrpc":"2.0","id":1,"method":"tools/list"}`)); !errors.Is(err, boom) {
		t.Fatalf("Send err = %v, want boom", err)
	}
	if _, err := rec.Recv(); !errors.Is(err, boom) {
		t.Fatalf("Recv err = %v, want boom", err)
	}
	if n := len(rec.Trace().Frames); n != 0 {
		t.Fatalf("recorded %d frames from a transport that accepted none", n)
	}
}
