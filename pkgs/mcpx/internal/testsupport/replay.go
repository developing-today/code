package testsupport

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"reflect"
	"sync"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

// ReplayTransport re-serves a recorded mcpclient.CallTrace to a fresh
// client: the replay half of #8242's family 2 (record-and-replay of
// tool calls). The client under replay must reproduce the recorded
// session frame for frame — every frame it sends is compared against
// the recording, and every frame it reads is the recording's next
// server frame, in the recorded order across both directions. A
// client that asks for something else, in some other order, has
// diverged from the captured scenario, and the replay fails loudly
// naming the frame where it happened rather than drifting into a
// session that merely resembles the original.
//
// Replay explores nothing: it re-runs what was recorded, with the
// recorded server's behaviour frozen at capture time. That is its
// purpose — a field failure or a flaky e2e becomes a deterministic
// regression test — and its limit, recorded in the #8242 evaluation.
//
// The interleave is strict. Send waits while the recording's next
// frame is a server frame the client has not yet consumed, and Recv
// waits while the next frame is a client frame not yet sent, because
// in the live session each frame genuinely followed the one before
// it. Waits end when the transport is closed or a divergence is
// recorded.
type ReplayTransport struct {
	mu     sync.Mutex
	cond   sync.Cond
	frames []mcpclient.CallFrame
	next   int
	err    error // first divergence; sticky once set
	closed bool
}

// NewReplayTransport serves trace's frames. The trace is not copied;
// do not mutate it while a replay is running.
func NewReplayTransport(trace *mcpclient.CallTrace) *ReplayTransport {
	r := &ReplayTransport{frames: trace.Frames}
	r.cond.L = &r.mu
	return r
}

// Send matches msg against the recording's next client frame.
func (r *ReplayTransport) Send(_ context.Context, msg []byte) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	for r.err == nil && !r.closed && r.next < len(r.frames) && r.frames[r.next].Dir == mcpclient.TraceRecv {
		r.cond.Wait()
	}
	if r.err != nil {
		return r.err
	}
	if r.closed {
		return errors.New("replay: transport closed")
	}
	if r.next >= len(r.frames) {
		return r.divergeLocked(fmt.Sprintf("client sent %s but the recording has no frames left", frameDesc(msg)))
	}
	want := r.frames[r.next]
	if !canonicalJSONEqual(want.Msg, msg) {
		return r.divergeLocked(fmt.Sprintf("frame %d: client sent %s, recording has %s",
			r.next, frameDesc(msg), frameDesc(want.Msg)))
	}
	r.next++
	r.cond.Broadcast()
	return nil
}

// Recv returns the recording's next server frame. It blocks while
// the next frame is a client frame the replay has not sent yet, and
// when the recording is exhausted — exactly as a live transport
// blocks when the server has said nothing — until Close.
func (r *ReplayTransport) Recv() ([]byte, error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	for r.err == nil && !r.closed && (r.next >= len(r.frames) || r.frames[r.next].Dir == mcpclient.TraceSend) {
		r.cond.Wait()
	}
	if r.err != nil {
		return nil, r.err
	}
	if r.closed {
		return nil, errors.New("replay: transport closed")
	}
	msg := append([]byte(nil), r.frames[r.next].Msg...)
	r.next++
	r.cond.Broadcast()
	return msg, nil
}

// Close ends the replay, unblocking any Send or Recv still waiting
// on the recorded interleave.
func (r *ReplayTransport) Close() error {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.closed = true
	r.cond.Broadcast()
	return nil
}

// Info describes the transport for diagnostics.
func (r *ReplayTransport) Info() string { return "replay" }

// Finish reports whether the replay consumed the whole recording.
// A session that ended early — the client never made the last
// recorded call — has not reproduced the scenario even if nothing
// diverged, so unconsumed frames are an error naming the next one
// owed. Call it after the client closes.
func (r *ReplayTransport) Finish() error {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.err != nil {
		return r.err
	}
	if r.next < len(r.frames) {
		f := r.frames[r.next]
		return fmt.Errorf("replay: %d recorded frame(s) never played; next owed is %s %s",
			len(r.frames)-r.next, f.Dir, frameDesc(f.Msg))
	}
	return nil
}

// divergeLocked records the first divergence and wakes every waiter
// so the failure surfaces on whichever call the client makes next,
// not only on the one that tripped it. Caller holds r.mu.
func (r *ReplayTransport) divergeLocked(msg string) error {
	if r.err == nil {
		r.err = fmt.Errorf("replay divergence: %s", msg)
	}
	r.cond.Broadcast()
	return r.err
}

// canonicalJSONEqual compares two frames by parsed meaning, not by
// bytes: key order and whitespace in a freshly marshalled frame may
// differ from the recording without the frame differing. Unparseable
// frames fall back to byte equality.
func canonicalJSONEqual(a, b []byte) bool {
	var av, bv any
	if json.Unmarshal(a, &av) != nil || json.Unmarshal(b, &bv) != nil {
		return bytes.Equal(a, b)
	}
	return reflect.DeepEqual(av, bv)
}

// frameDesc names a frame for divergence reports: its method and id,
// or the id alone for a response.
func frameDesc(msg []byte) string {
	var f struct {
		ID     *int64 `json:"id"`
		Method string `json:"method"`
	}
	_ = json.Unmarshal(msg, &f)
	switch {
	case f.Method != "" && f.ID != nil:
		return fmt.Sprintf("request %q id=%d", f.Method, *f.ID)
	case f.Method != "":
		return fmt.Sprintf("notification %q", f.Method)
	case f.ID != nil:
		return fmt.Sprintf("response id=%d", *f.ID)
	default:
		return "an unparseable frame"
	}
}
