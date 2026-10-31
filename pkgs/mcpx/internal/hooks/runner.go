package hooks

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/dezren39/mcpx/internal/defaults"
	"github.com/dezren39/mcpx/internal/events"
)

// hookOutputLimit bounds how much of a hook's combined stdout+stderr is
// retained in daemon memory (defaults.ExecStderrLimit). Output is still
// drained, so excess bytes are discarded, not left blocking the pipes.
const hookOutputLimit = 65536

// Stats is one hook's counters since the runner started. Doctor and
// status read these; a hook that is slow or failing is visible here
// and in the hook.failed events the runner publishes.
type Stats struct {
	Name           string `json:"name"`
	Fired          uint64 `json:"fired"`
	Failed         uint64 `json:"failed"`
	FailedClosed   uint64 `json:"failedClosed"`
	TimedOut       uint64 `json:"timedOut"`
	Dropped        uint64 `json:"dropped"`
	Slow           uint64 `json:"slow"`
	LastDurationMs int64  `json:"lastDurationMs"`
	LastError      string `json:"lastError,omitempty"`
}

// Runner subscribes to the bus once per declared hook, using each
// hook's own Filter, and runs one serial worker per hook.
type Runner struct {
	bus   *events.Bus
	logf  func(string, ...any)
	hooks []Hook

	mu    sync.Mutex
	stats map[string]*Stats
	subs  []*events.Subscription

	ctx    context.Context
	cancel context.CancelFunc
	wg     sync.WaitGroup
}

// NewRunner builds a runner. logf may be nil.
func NewRunner(bus *events.Bus, hs []Hook, logf func(string, ...any)) *Runner {
	if logf == nil {
		logf = func(string, ...any) {}
	}
	r := &Runner{bus: bus, logf: logf, hooks: hs, stats: map[string]*Stats{}}
	for _, h := range hs {
		r.stats[h.Name] = &Stats{Name: h.Name}
	}
	return r
}

// Hooks returns the declarations in force.
func (r *Runner) Hooks() []Hook { return r.hooks }

// Stats returns a snapshot of every hook's counters.
func (r *Runner) Stats() []Stats {
	r.mu.Lock()
	defer r.mu.Unlock()
	out := make([]Stats, 0, len(r.hooks))
	for i, h := range r.hooks {
		st := *r.stats[h.Name]
		if i < len(r.subs) && r.subs[i] != nil {
			st.Dropped = r.subs[i].Dropped.Load()
		}
		out = append(out, st)
	}
	return out
}

// Start subscribes and launches the workers. Stop ends them.
// Each hook subscribes with its own Filter, so drops are counted
// accurately per hook and never misattributed across differing filters.
func (r *Runner) Start() {
	r.ctx, r.cancel = context.WithCancel(context.Background())
	r.subs = make([]*events.Subscription, 0, len(r.hooks))
	for _, h := range r.hooks {
		sub, _ := r.bus.Subscribe(h.SubFilter(), 0)
		r.subs = append(r.subs, sub)
		r.wg.Add(1)
		go r.work(h, sub)
	}
}

// Stop unsubscribes and waits for the workers to drain their current
// execution (each still bounded by its own timeout).
func (r *Runner) Stop() {
	for _, sub := range r.subs {
		if sub != nil {
			sub.Close()
		}
	}
	if r.cancel != nil {
		r.cancel()
	}
	r.wg.Wait()
}

func (r *Runner) work(h Hook, sub *events.Subscription) {
	defer r.wg.Done()
	var lastDropped uint64
	for {
		select {
		case <-r.ctx.Done():
			return
		case e, ok := <-sub.C:
			if !ok {
				return
			}
			// The runner's own voice is never a hook trigger.
			if strings.HasPrefix(string(e.Kind), "hook.") {
				continue
			}
			droppedNow := sub.Dropped.Load()
			gapNow := droppedNow - lastDropped
			lastDropped = droppedNow
			if gapNow > 0 {
				r.mu.Lock()
				r.stats[h.Name].Dropped = droppedNow
				r.mu.Unlock()
			}
			r.execute(h, e, gapNow)
		}
	}
}

// limitedHookOutput returns the bounded writer for one hook execution:
// the daemon's ExecStderrLimit where configured, else hookOutputLimit.
func limitedHookOutput() *cappedWriter {
	max := defaults.ExecStderrLimit
	if max <= 0 {
		max = hookOutputLimit
	}
	return &cappedWriter{max: max}
}

// payload is the JSON a hook receives on stdin: the event itself, plus
// the runner's delivery facts (which hook, and how many events were
// dropped for it since the last delivery).
type payload struct {
	events.Event
	Hook    string `json:"hook"`
	Dropped uint64 `json:"dropped,omitempty"`
}

// execute runs one hook for one event, under the hook's hard timeout.
// A failure is a result, not an error (the adapter's precedent): it is
// counted, logged, and published as hook.failed; the daemon and the
// operation the hook watched carry on either way.
func (r *Runner) execute(h Hook, e events.Event, dropped uint64) {
	body, err := json.Marshal(payload{Event: e, Hook: h.Name, Dropped: dropped})
	if err != nil {
		return
	}
	argv, err := ArgvFor(h)
	if err != nil {
		r.recordFailure(h, e, 0, err)
		return
	}
	ctx, cancel := context.WithTimeout(r.ctx, h.Timeout)
	defer cancel()
	cmd := exec.CommandContext(ctx, argv[0], argv[1:]...)
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	cmd.Cancel = func() error {
		if cmd.Process != nil {
			_ = syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL)
		}
		return cmd.Process.Kill()
	}
	cmd.Stdin = bytes.NewReader(body)
	cmd.Env = append(os.Environ(),
		"MCPX_HOOK_NAME="+h.Name,
		"MCPX_HOOK_PHASE=observe",
		fmt.Sprintf("MCPX_HOOK_EVENT_KIND=%s", e.Kind),
		fmt.Sprintf("MCPX_HOOK_EVENT_SEQ=%d", e.Seq),
		"MCPX_HOOK_EVENT_SERVER="+e.Server,
		"MCPX_HOOK_EVENT_SESSION="+e.Session,
		"MCPX_HOOK_EVENT_URI="+e.URI,
	)
	var out = limitedHookOutput()
	cmd.Stdout = out
	cmd.Stderr = out
	start := time.Now()
	if err := cmd.Start(); err != nil {
		r.recordFailure(h, e, 0, err)
		return
	}
	pgid := cmd.Process.Pid
	done := make(chan struct{})
	defer close(done)
	go func() {
		select {
		case <-done:
		case <-ctx.Done():
			_ = syscall.Kill(-pgid, syscall.SIGKILL)
		}
	}()
	runErr := cmd.Wait()
	dur := time.Since(start)

	r.mu.Lock()
	st := r.stats[h.Name]
	st.Fired++
	st.LastDurationMs = dur.Milliseconds()
	if dur >= h.Timeout*4/5 && runErr == nil {
		st.Slow++
	}
	r.mu.Unlock()

	if ctx.Err() == context.DeadlineExceeded {
		r.mu.Lock()
		r.stats[h.Name].TimedOut++
		r.mu.Unlock()
		r.recordFailure(h, e, dur, fmt.Errorf("hook timed out after %s", h.Timeout))
		return
	}
	if runErr != nil {
		r.recordFailure(h, e, dur, fmt.Errorf("%v: %s", runErr, firstLine(out.String())))
		return
	}
	if st := r.statsFor(h.Name); st.Slow > 0 && dur >= h.Timeout*4/5 {
		r.logf("hooks: %s slow on %s: %s of %s budget", h.Name, e.Kind, dur.Round(time.Millisecond), h.Timeout)
	}
}

func (r *Runner) statsFor(name string) Stats {
	r.mu.Lock()
	defer r.mu.Unlock()
	return *r.stats[name]
}

func (r *Runner) recordFailure(h Hook, e events.Event, dur time.Duration, err error) {
	r.mu.Lock()
	st := r.stats[h.Name]
	st.Failed++
	if h.FailureClosed {
		st.FailedClosed++
	}
	st.LastError = err.Error()
	r.mu.Unlock()
	r.logf("hooks: %s failed on %s: %v", h.Name, e.Kind, err)
	data, merr := json.Marshal(map[string]any{
		"hook": h.Name, "kind": string(e.Kind), "seq": e.Seq,
		"durationMs": dur.Milliseconds(), "error": err.Error(),
		"failure": map[bool]string{true: "closed", false: "open"}[h.FailureClosed],
	})
	if merr == nil && r.bus != nil {
		r.bus.Publish(events.Event{Kind: events.HookFailed, Server: e.Server, Session: e.Session, Data: data})
	}
}

// argvFor resolves a hook's action to a command line. It is kept for
// external callers that already name it; new code prefers ArgvFor.
func argvFor(h Hook) ([]string, error) {
	return ArgvFor(h)
}

// cappedWriter keeps a bounded head of hook output: enough to say why
// it failed, bounded so a chatty hook cannot exhaust daemon memory.
// Excess bytes are drained and discarded, never retained.
type cappedWriter struct {
	mu  sync.Mutex
	buf bytes.Buffer
	max int
}

func (w *cappedWriter) Write(p []byte) (int, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	if room := w.max - w.buf.Len(); room > 0 {
		if len(p) > room {
			w.buf.Write(p[:room])
		} else {
			w.buf.Write(p)
		}
	}
	return len(p), nil
}

func (w *cappedWriter) String() string {
	w.mu.Lock()
	defer w.mu.Unlock()
	return w.buf.String()
}

func firstLine(s string) string {
	s = strings.TrimSpace(s)
	if i := strings.IndexByte(s, '\n'); i >= 0 {
		return s[:i]
	}
	return s
}
