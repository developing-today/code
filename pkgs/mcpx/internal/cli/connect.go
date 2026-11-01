package cli

import (
	"context"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/daemon"
)

// Attempt is one rung of the connection ladder, and what happened on it.
type Attempt struct {
	How    string `json:"how"`
	Target string `json:"target,omitempty"`
	OK     bool   `json:"ok"`
	Detail string `json:"detail,omitempty"`
	TookMs int64  `json:"tookMs"`
}

// Ladder is the ordered set of ways to reach a daemon.
//
// Ordered by cost, cheapest first, because the cheap ones are also the ones
// that usually work:
//
//	socket      0.17ms warm    an already-running daemon on this machine
//	url         0.65ms warm    one somewhere else, when configured
//	spawn      ~23ms once      start one, then use its socket
//	inline        n/a          no daemon at all
//
// The last rung matters more than it looks. A daemon is a cache and a
// process pool; neither is required to answer a question. When one cannot be
// started -- a read-only filesystem, a sandbox with no fork, a container
// whose init will not reap -- mcpx can run the servers in its own process
// and exit when it is done. Slower every time, but it works where nothing
// else does.
type Ladder struct {
	Attempts []Attempt `json:"attempts"`
	// Chosen is the rung that worked.
	Chosen string `json:"chosen"`
}

// ConnectOptions control the ladder.
type ConnectOptions struct {
	// Endpoint, when set, is tried instead of the socket and spawn rungs.
	// Naming a daemon means that daemon, not one like it.
	Endpoint string
	// AllowSpawn permits starting a daemon.
	AllowSpawn bool
	// AllowInline permits running servers in this process.
	AllowInline bool
	// SpawnTimeout bounds how long to wait for a spawned daemon to answer.
	SpawnTimeout time.Duration
}

// Connect walks the ladder and returns the first client that answers.
//
// Every rung is recorded whether or not it worked, because "why is this
// slow" and "why did it use the wrong daemon" are both answered by the list
// of what was tried.
func (a *App) Connect(ctx context.Context, opt ConnectOptions) (*Client, *Ladder, error) {
	// The socket is keyed to the configuration, so that two projects get two
	// daemons rather than one serving the wrong servers. Using the unkeyed
	// default here looked for a socket nothing ever creates.
	paths, _ := a.resolvePathsForConfig()

	l := &Ladder{}
	record := func(how, target string, start time.Time, err error) bool {
		at := Attempt{
			How: how, Target: target, OK: err == nil,
			TookMs: time.Since(start).Milliseconds(),
		}
		if err != nil {
			at.Detail = err.Error()
		}
		l.Attempts = append(l.Attempts, at)
		return err == nil
	}

	// A named endpoint short-circuits the ladder. Falling back to a local
	// daemon when the named one is unreachable would answer from the wrong
	// machine, which is worse than failing.
	if opt.Endpoint != "" {
		start := time.Now()
		c := NewClientAt(paths, a.ConfigPath, opt.Endpoint)
		if c.reachable(ctx) {
			record("url", opt.Endpoint, start, nil)
			l.Chosen = "url"
			return c, l, nil
		}
		err := fmt.Errorf("no daemon answering")
		record("url", opt.Endpoint, start, err)
		return nil, l, fmt.Errorf("%s: %w; it is named explicitly, so mcpx will not "+
			"start a local one in its place", opt.Endpoint, err)
	}

	// Rung one: a socket already listening.
	start := time.Now()
	c := NewClient(paths, a.ConfigPath)
	if c.Ping(ctx) {
		record("socket", paths.Socket, start, nil)
		l.Chosen = "socket"
		return c, l, nil
	}
	record("socket", paths.Socket, start, fmt.Errorf("nothing listening"))

	// Rung two: any other socket in the state directory. A daemon started
	// for a slightly different configuration is still a daemon, and asking
	// it is cheaper than starting another -- but only if it agrees about
	// what it is serving, which Ping does not tell us. So this rung is
	// reported and not taken automatically.
	if others := siblingSockets(paths); len(others) > 0 {
		l.Attempts = append(l.Attempts, Attempt{
			How: "sibling-socket", OK: false,
			Detail: fmt.Sprintf("%d other daemon(s) here, for other configurations: %s",
				len(others), strings.Join(others, ", ")),
		})
	}

	// Rung three: start one.
	if opt.AllowSpawn {
		start = time.Now()
		err := c.EnsureDaemon(ctx)
		if record("spawn", paths.Socket, start, err) {
			l.Chosen = "spawn"
			return c, l, nil
		}
	} else {
		l.Attempts = append(l.Attempts, Attempt{
			How: "spawn", OK: false, Detail: "disabled by daemon.autostart",
		})
	}

	// Rung four: no daemon at all.
	if opt.AllowInline {
		start = time.Now()
		inline, err := a.inlineDaemon(ctx)
		if record("inline", "in-process", start, err) {
			l.Chosen = "inline"
			return inline, l, nil
		}
	}

	return nil, l, fmt.Errorf("no daemon reachable:\n%s", l.Describe())
}

// Describe renders the attempts, for an error or for `mcpx doctor`.
func (l *Ladder) Describe() string {
	var b strings.Builder
	for _, at := range l.Attempts {
		mark := "  -"
		if at.OK {
			mark = "  *"
		}
		fmt.Fprintf(&b, "%s %-16s %-44s %s\n", mark, at.How, trunc(at.Target, 44), at.Detail)
	}
	return b.String()
}

// probeTimeout bounds how long a stranger's socket is given to answer.
//
// Short by design: the listing probes each daemon in turn, and one dead
// socket should not hold up the rest of the answer.
func probeTimeout() time.Duration {
	return clientSettings().Duration("daemon.probeTimeout")
}

// siblingSockets lists other daemons in the same state directory.
func siblingSockets(paths daemon.Paths) []string {
	dir := filepath.Dir(paths.Socket)
	entries, err := os.ReadDir(dir)
	if err != nil {
		return nil
	}
	var out []string
	for _, e := range entries {
		name := e.Name()
		if !strings.HasSuffix(name, ".sock") || filepath.Join(dir, name) == paths.Socket {
			continue
		}
		// Only count one that is actually listening; a stale socket file
		// outlives the process that made it.
		conn, err := net.DialTimeout("unix", filepath.Join(dir, name), probeTimeout())
		if err != nil {
			continue
		}
		_ = conn.Close()
		out = append(out, name)
	}
	sort.Strings(out)
	return out
}

// inlineDaemon starts a daemon inside this process.
//
// The last resort, and it changes the lifetime of everything: servers start
// when first called and die when this process exits, so nothing is pooled
// across invocations and a stateful server cannot outlive one command.
//
// Worth having anyway. A daemon is an optimisation -- a cache and a process
// pool -- and neither is required to answer a question. Where one cannot be
// started, this is the difference between mcpx working slowly and not
// working.
// inlineDaemonPing is Client.Ping, indirected so tests can keep the
// in-process daemon from ever answering and so drive inlineDaemon's poll
// loop deterministically: a daemon with an empty configuration answers
// the first Ping, so the wait below is otherwise never entered (#323).
// Production code never reassigns it.
var inlineDaemonPing = (*Client).Ping

func (a *App) inlineDaemon(ctx context.Context) (*Client, error) {
	// A cancelled caller must not be read as "no daemon answered": Ping is
	// ctx-bound, so it could never succeed, and the poll below would burn
	// the whole inline start timeout before reporting the wrong error.
	// EnsureDaemon learned this in #233; this rung gets the same checks.
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	cfg, err := config.Load(a.ConfigPath)
	if err != nil {
		return nil, fmt.Errorf("inline mode needs a configuration: %w", err)
	}
	config.ApplyPoolSettings(cfg, a.Settings())
	// A private socket in a private directory, so two inline runs in the
	// same shell cannot find each other's servers.
	dir, err := os.MkdirTemp("", "mcpx-inline-")
	if err != nil {
		return nil, err
	}
	paths := a.Paths
	paths.Socket = filepath.Join(dir, "d.sock")
	a.daemonSocket = paths.Socket

	// A discarding logger. A daemon's operational chatter belongs in its
	// log file; in-process it would land in the middle of the command's own
	// output, which is the one place it must not go.
	srv, err := daemon.NewServer(daemon.Options{
		Config: cfg, Paths: paths, Version: a.Version,
		Logger:   log.New(io.Discard, "", 0),
		Settings: a.Settings(),
		Augment:  a.augmentConfig,
	})
	if err != nil {
		_ = os.RemoveAll(dir)
		return nil, err
	}
	// Tied to the process rather than to ctx. The caller's context is for
	// the connection attempt; the servers have to live as long as the
	// command using them does.
	life, stop := context.WithCancel(context.Background())
	// The API is hosted in this process and the client carries its
	// requests to the daemon's handler through an in-process transport:
	// inline mode no longer serialises every call through a socket to
	// itself. The socket is opened only if a script asks for it, via
	// the client's lazy socket: a script is a child process, and a
	// path is the only way it can reach this daemon.
	transport := newInProcessTransport(srv.InProcessHandler(life))
	a.inlineStop = func() {
		stop()
		transport.Close()
		_ = os.RemoveAll(dir)
	}
	c := NewInProcessClient(paths, a.ConfigPath, transport, func() (string, error) {
		if err := srv.ServeSocket(life, paths.Socket); err != nil {
			return "", err
		}
		return paths.Socket, nil
	})
	deadline := time.Now().Add(a.Settings().Duration("daemon.inlineStartTimeout"))
	for time.Now().Before(deadline) {
		if inlineDaemonPing(c, ctx) {
			return c, nil
		}
		select {
		case <-ctx.Done():
			a.inlineStop()
			return nil, ctx.Err()
		case <-time.After(a.Settings().Duration("daemon.inlineStartPoll")):
		}
	}
	if err := ctx.Err(); err != nil {
		a.inlineStop()
		return nil, err
	}
	a.inlineStop()
	return nil, fmt.Errorf("the in-process daemon did not answer")
}

// inProcessTransport carries HTTP requests to a handler hosted in this
// process. Each request travels over a fresh net.Pipe connection served
// by a private http.Server, so streaming responses and the handler's
// own connection semantics are the ones a socket would give it — only
// the kernel, the socket file, and the serialisation round trip to a
// daemon that is this process are gone.
type inProcessTransport struct {
	inner *http.Transport
	srv   *http.Server
	ln    *pipeListener
}

func newInProcessTransport(handler http.Handler) *inProcessTransport {
	ln := newPipeListener()
	t := &inProcessTransport{ln: ln, srv: &http.Server{Handler: handler}}
	go func() { _ = t.srv.Serve(ln) }()
	t.inner = &http.Transport{
		DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
			client, server := net.Pipe()
			if err := ln.deliver(ctx, server); err != nil {
				_ = client.Close()
				_ = server.Close()
				return nil, err
			}
			return client, nil
		},
		// One pipe per request, as the socket client makes one
		// connection per request: a takeover or a stop can never strand
		// a call on a connection the other end has finished with.
		DisableKeepAlives: true,
	}
	return t
}

// RoundTrip implements http.RoundTripper.
func (t *inProcessTransport) RoundTrip(req *http.Request) (*http.Response, error) {
	return t.inner.RoundTrip(req)
}

// Close stops the private server and refuses new connections.
func (t *inProcessTransport) Close() {
	_ = t.srv.Close()
	t.ln.Close()
	t.inner.CloseIdleConnections()
}

// pipeListener is a net.Listener whose connections arrive by hand-off
// from the transport's dialler instead of from a kernel socket.
type pipeListener struct {
	conns chan net.Conn
	done  chan struct{}
	once  sync.Once
}

func newPipeListener() *pipeListener {
	return &pipeListener{conns: make(chan net.Conn), done: make(chan struct{})}
}

func (l *pipeListener) deliver(ctx context.Context, c net.Conn) error {
	select {
	case l.conns <- c:
		return nil
	case <-l.done:
		return net.ErrClosed
	case <-ctx.Done():
		return ctx.Err()
	}
}

func (l *pipeListener) Accept() (net.Conn, error) {
	select {
	case c := <-l.conns:
		return c, nil
	case <-l.done:
		return nil, net.ErrClosed
	}
}

func (l *pipeListener) Close() error {
	l.once.Do(func() { close(l.done) })
	return nil
}

type pipeAddr struct{}

func (pipeAddr) Network() string { return "in-process" }
func (pipeAddr) String() string  { return "in-process" }

func (l *pipeListener) Addr() net.Addr { return pipeAddr{} }

// CloseInline stops an in-process daemon, if one was started.
func (a *App) CloseInline() {
	if a.inlineStop != nil {
		a.inlineStop()
		a.inlineStop = nil
	}
}
