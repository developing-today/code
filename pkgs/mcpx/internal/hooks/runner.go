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
	"time"

	"github.com/dezren39/mcpx/internal/events"
)

// queueDepth bounds one hook's backlog. A hook that falls behind loses
// events rather than memory: the loss is counted (Stats.Dropped) and
// the next event the hook does receive carries the gap count, which is
// #6's gap contract in miniature -- one explicit marker, never a
// silent hole.
const queueDepth = 32

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

// Runner subscribes to the bus once and fans events out to one serial
// worker per declared hook, so ordering within a hook is promised and
// ordering across hooks is not (decision 0001, delivery semantics).
type Runner struct {
	bus   *events.Bus
	logf  func(string, ...any)
	hooks []Hook

	mu    sync.Mutex
	stats map[string]*Stats
	subs  *events.Subscription

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
	for _, h := range r.hooks {
		out = append(out, *r.stats[h.Name])
	}
	return out
}

// Start subscribes and launches the workers. Stop ends them.
func (r *Runner) Start() {
	r.ctx, r.cancel = context.WithCancel(context.Background())
	sub, _ := r.bus.Subscribe(events.Filter{}, 0)
	r.subs = sub
	queues := make(map[string]chan events.Event, len(r.hooks))
	for _, h := range r.hooks {
		q := make(chan events.Event, queueDepth)
		queues[h.Name] = q
		r.wg.Add(1)
		go r.work(h, q)
	}
	r.wg.Add(1)
	go func() {
		defer r.wg.Done()
		for e := range sub.C {
			// The runner's own voice is never a hook trigger.
			if strings.HasPrefix(string(e.Kind), "hook.") {
				continue
			}
			for _, h := range r.hooks {
				if !h.Matches(e) {
					continue
				}
				select {
				case queues[h.Name] <- e:
				default:
					r.mu.Lock()
					r.stats[h.Name].Dropped++
					r.mu.Unlock()
				}
			}
		}
	}()
}

// Stop unsubscribes and waits for the workers to drain their current
// execution (each still bounded by its own timeout).
func (r *Runner) Stop() {
	if r.subs != nil {
		r.subs.Close()
	}
	if r.cancel != nil {
		r.cancel()
	}
	r.wg.Wait()
}

func (r *Runner) work(h Hook, q chan events.Event) {
	defer r.wg.Done()
	var gap uint64
	for {
		select {
		case <-r.ctx.Done():
			return
		case e := <-q:
			r.mu.Lock()
			gapNow := r.stats[h.Name].Dropped - gap
			gap = r.stats[h.Name].Dropped
			r.mu.Unlock()
			r.execute(h, e, gapNow)
		}
	}
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
	argv, err := argvFor(h)
	if err != nil {
		r.recordFailure(h, e, 0, err)
		return
	}
	ctx, cancel := context.WithTimeout(r.ctx, h.Timeout)
	defer cancel()
	cmd := exec.CommandContext(ctx, argv[0], argv[1:]...)
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
	var out bytes.Buffer
	cmd.Stdout = &out
	cmd.Stderr = &out
	start := time.Now()
	runErr := cmd.Run()
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

// argvFor resolves a hook's action to a command line.
//
// run is verbatim: the declaration is the argv, never a shell string.
// A JS module runs under the first JS runtime on PATH, in the order
// the script settings try them (deno, bun, node). A Python module runs
// under python3, or `uv run` when python3 is absent -- Python is a
// module kind here because #1872 names it one, resolved in this one
// place so the choice is made once.
func argvFor(h Hook) ([]string, error) {
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

func firstLine(s string) string {
	s = strings.TrimSpace(s)
	if i := strings.IndexByte(s, '\n'); i >= 0 {
		return s[:i]
	}
	return s
}
