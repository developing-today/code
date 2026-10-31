package daemon

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log"
	"net"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"syscall"
	"time"

	"github.com/dezren39/mcpx/internal/api"
	"github.com/dezren39/mcpx/internal/codegen"
	"github.com/dezren39/mcpx/internal/config"
	"github.com/dezren39/mcpx/internal/defaults"
	"github.com/dezren39/mcpx/internal/elicit"
	"github.com/dezren39/mcpx/internal/events"
	"github.com/dezren39/mcpx/internal/execsvc"
	"github.com/dezren39/mcpx/internal/hooks"
	"github.com/dezren39/mcpx/internal/logging"
	"github.com/dezren39/mcpx/internal/logstore"
	"github.com/dezren39/mcpx/internal/mcpclient"
	"github.com/dezren39/mcpx/internal/mcpserver"
	"github.com/dezren39/mcpx/internal/pool"
	"github.com/dezren39/mcpx/internal/settings"
	"github.com/dezren39/mcpx/internal/tasks"
)

// Server is the local HTTP API.
//
// It listens on a unix socket (for the CLI, no port to collide with) and on a
// loopback TCP port (for generated script clients, because fetch() over a unix
// socket is not portable across Deno, Bun and Node).
type Server struct {
	reg *Registry
	// augment is Options.Augment, kept for reloads.
	augment func(*config.Config) error
	paths   Paths
	cfg     *config.Config
	logger  *log.Logger

	// Address is the interface the TCP listener binds. Empty is loopback.
	Address string

	// MCP is mcpx's own MCP server, mounted on the daemon's listeners.
	//
	// Supplied from outside because the daemon cannot import the CLI that
	// builds it -- the dependency runs one way, and inverting it to put the
	// two HTTP servers together would be a worse cure than the disease.
	// Nil means the daemon serves /v1 only.
	MCP http.Handler
	// MCPPath is where MCP is mounted.
	MCPPath string
	// MCPTool invokes one of mcpx's own MCP tools by name, for the plain
	// POST projection of it. Supplied alongside MCP, for the same reason.
	MCPTool func(ctx context.Context, tool string, args json.RawMessage) (string, error)

	// Events carries everything the daemon notices, to every subscriber.
	Events *events.Bus

	// hookRunner runs the config-declared hooks (the `hooks` setting)
	// over Events. Nil when none are declared, or when the declaration
	// failed to parse -- doctor reports that case, and a daemon that
	// refuses to start over an observer's typo helps nobody.
	hookRunner *hooks.Runner

	// Origins decides which browser origins may use the daemon at all; see
	// refuseBrowserPages. The zero value allows loopback origins only.
	Origins mcpserver.OriginPolicy

	httpSrv *http.Server
	// httpServing counts the serve loops of httpSrv, so a drain can wait for
	// them to exit. See passConns.
	httpServing sync.WaitGroup
	// tcpLn and unixLn are *connListener once Listen has run. They are the
	// daemon's only accept path, so a handover can see every connection.
	tcpLn  net.Listener
	unixLn net.Listener
	// tsnetLn is set only when daemon.tsnet.hostname names a tailnet
	// identity for this daemon (#1967); nil on every daemon that did not
	// ask for one.
	tsnetLn net.Listener
	// conns tracks every connection the HTTP server holds, for a handover.
	conns    connTracker
	endpoint string
	version  string
	started  time.Time

	// httpErr carries a failure from either HTTP server to Serve. Every server
	// the daemon starts reports here, including one restored after a takeover
	// that did not commit.
	httpErr chan error
	// takeoverMu admits one takeover at a time. committed is set once a
	// successor has accepted the daemon's children, after which this process
	// must not stop, since it still owns the child stdin pipes until the
	// successor hangs up. handedOff closes when Serve may return.
	takeoverMu    sync.Mutex
	committed     atomic.Bool
	handedOff     chan struct{}
	handedOffOnce sync.Once
	// adoption is the Adoption of the takeover this daemon committed, kept
	// so an orderly exit can wait for the background orphan-session
	// terminations Attach started (#298). Nil when this daemon never took
	// over; an aborted takeover stores nothing, since it sent no DELETEs.
	adoption atomic.Pointer[Adoption]
	// upgradeMu admits one upgrade at a time. upgrading is the upgrade whose
	// successor is running, which the successor's takeover answers.
	upgradeMu sync.Mutex
	upgrading atomic.Pointer[upgradeRun]
	// ConfigArg is the --config the daemon was started with, which an
	// upgrade's successor is given too so that both load the same file.
	ConfigArg string

	// idleExit stops a daemon that nobody has used for this long and that
	// holds no live MCP instances. Auto-started daemons set it so that a repo
	// visited once does not leave a process behind forever; a daemon run under
	// launchd leaves it at zero and stays up.
	idleExit time.Duration

	// sink is the durable log POST /v1/log appends to. Nil when the log
	// could not be opened, which the daemon survives and the route reports.
	sink    *logging.FileSink
	lastReq atomic.Int64

	// cfgMu guards cfg, contentHash and stamp. The daemon re-reads its
	// configuration from a request goroutine now, so what used to be
	// write-once state is not.
	cfgMu sync.RWMutex
	// reloadMu serialises reloads, so two requests arriving together do not
	// each rebuild the pool map from the same starting point.
	reloadMu sync.Mutex
	// contentHash is what the configuration files said when they were last
	// read, so an edit made behind the daemon's back is noticed rather than
	// silently ignored until a restart.
	contentHash string
	// stamp is the cheap version of the same question: modification times
	// and sizes, so the common case costs a stat rather than a read.
	stamp string

	// set is the daemon's resolved configuration. Read on every request
	// rather than copied into fields at startup, so a runtime change through
	// PUT /v1/settings reaches the code that acts on it without a restart.
	set *settings.Set

	// tasks holds calls a client asked to run in the background. Created on
	// first use, because a daemon that never runs one should carry nothing.
	taskOnce sync.Once
	tasks    *tasks.Store

	// calls is the exec call-frame registry shared between the exec
	// services this daemon builds and its /v1/call handlers (#1927).
	// Created on first use, like tasks.
	callOnce sync.Once
	calls    *execsvc.CallRegistry

	tokenStore *logstore.TokenStore

	// consumer holds the policy and the schema history behind the things
	// mcpx asks for itself. See internal/daemon/consumer.go.
	consumer *consumerState
	artifactState
}

// Options configure the daemon.
type Options struct {
	Config  *config.Config
	Paths   Paths
	Version string
	// TCPPort of 0 picks a free ephemeral port.
	TCPPort int
	Logger  *log.Logger
	// Warm fetches all schemas at startup instead of on first use.
	Warm bool
	// IdleExit stops an unused daemon after this long. Zero means never.
	IdleExit time.Duration
	// Sink is the durable log the daemon already writes; POST /v1/log
	// appends to the same one rather than opening a second writer.
	Sink *logging.FileSink
	// Settings is the resolved configuration. Nil means the built-in
	// defaults, which is what a test that only wants a daemon should get
	// rather than a nil dereference.
	Settings *settings.Set
	// Augment adds servers that are not written in a config file -- an
	// adapted program is one -- to every config the daemon loads, at start
	// and on each reload, so they go through the same pools, cache, codegen
	// and /v1 routes as a configured server. Nil adds nothing.
	Augment func(*config.Config) error
}

// NewServer builds the daemon but does not listen yet.
func NewServer(opts Options) (*Server, error) {
	if opts.Logger == nil {
		opts.Logger = log.New(os.Stderr, "", log.LstdFlags|log.Lmicroseconds)
	}
	if err := opts.Paths.EnsureDirs(); err != nil {
		return nil, err
	}
	if opts.Augment != nil && opts.Config != nil {
		if err := opts.Augment(opts.Config); err != nil {
			return nil, err
		}
	}
	reg, err := NewRegistry(opts.Config, opts.Paths, func(f string, a ...any) {
		opts.Logger.Printf(f, a...)
	})
	if err != nil {
		return nil, err
	}
	if opts.Settings == nil {
		sch, serr := settings.New(settings.Registry())
		if serr != nil {
			return nil, serr
		}
		opts.Settings = settings.NewSet(sch)
	}
	if err := ValidateBearerToken(opts.Settings); err != nil {
		return nil, err
	}
	// logging.trace, which is MCPX_TRACE. It was read here by name, so a
	// configuration file or `mcpx daemon --logging-trace` could not turn it on.
	if opts.Settings.Bool("logging.trace") {
		pool.Trace = func(f string, a ...any) { opts.Logger.Printf(f, a...) }
	}
	srv := &Server{
		set:       opts.Settings,
		reg:       reg,
		paths:     opts.Paths,
		cfg:       opts.Config,
		logger:    opts.Logger,
		version:   opts.Version,
		idleExit:  opts.IdleExit,
		sink:      opts.Sink,
		augment:   opts.Augment,
		Events:    events.New(opts.Settings.Int("events.history")),
		httpErr:   make(chan error, 4),
		handedOff: make(chan struct{}),
	}
	// The broker is optional: a daemon whose state directory cannot hold a
	// database still serves tools, and answers every server question with
	// cancel rather than hanging.
	broker, berr := OpenBroker(opts.Paths)
	if berr != nil {
		opts.Logger.Printf("elicitation disabled: %v", berr)
		broker = nil
	}
	if broker != nil && opts.Settings != nil {
		// The declared elicit.ttl / elicit.maxAge settings govern the
		// broker's touch-on-read convention; without this the registry
		// entries would be promises nothing honours.
		broker.Configure(opts.Settings.Duration("elicit.ttl"),
			opts.Settings.Duration("elicit.maxAge"))
	}
	reg.UseSettings(opts.Settings)
	reg.InstallHooks(srv.Events, broker, nil)
	if raw := opts.Settings.String("hooks"); raw != "" {
		declared, herr := hooks.Parse(raw, opts.Settings.Duration("hooks.timeout"))
		if herr != nil {
			opts.Logger.Printf("hooks: %v (no hooks installed)", herr)
		} else if len(declared) > 0 {
			srv.hookRunner = hooks.NewRunner(srv.Events, declared, func(f string, a ...any) {
				opts.Logger.Printf(f, a...)
			})
			srv.hookRunner.Start()
		}
	}
	srv.initConsumer()
	srv.tokenStore, _ = logstore.NewTokenStore(filepath.Join(opts.Paths.State, "tokens.db"))
	if opts.Config != nil {
		srv.contentHash = ContentFingerprint(opts.Config.Sources)
		srv.stamp = stampConfig(opts.Config.Sources)
	}
	srv.lastReq.Store(time.Now().UnixNano())
	return srv, nil
}

// Registry exposes the pool registry (used by the in-process fast path).
func (s *Server) Registry() *Registry { return s.reg }

// HookRunner exposes the declared-hook runner, for status and tests.
// Nil when no hooks are declared.
func (s *Server) HookRunner() *hooks.Runner { return s.hookRunner }

// Endpoint is the loopback base URL once Listen has run.
func (s *Server) Endpoint() string { return s.endpoint }

// TSNetStatus reports this daemon's tailnet identity and, per peer,
// whether traffic is direct or relayed. ok is false on a daemon with no
// tsnet listener.
func (s *Server) TSNetStatus(ctx context.Context) (status TSNetStatus, ok bool, err error) {
	cl, isConn := s.tsnetLn.(*connListener)
	if !isConn || cl == nil {
		return TSNetStatus{}, false, nil
	}
	tl, isTSNet := cl.inner.(*tsnetListener)
	if !isTSNet {
		return TSNetStatus{}, false, nil
	}
	status, err = tl.Status(ctx)
	if err != nil {
		return TSNetStatus{}, true, err
	}
	return status, true, nil
}

// Listen binds both listeners and publishes the daemon record.
func (s *Server) Listen(tcpPort int) error {
	// The bind call site for MCPX_SOCKET (#1886 step 6): a daemon started
	// with the variable binds the named socket, and the record it
	// publishes then names that socket.
	s.paths.Socket = s.paths.BindSocket()
	// The selected bind path's parent may not exist yet (#1886, Codex
	// on PR #2561): NewServer ran EnsureDirs against the *computed*
	// socket, so an MCPX_SOCKET pointing into a fresh directory would
	// fail to bind. Create the bind path's parent here, at the bind
	// call site, once the bind socket is actually known.
	if dir := filepath.Dir(s.paths.Socket); dir != "" {
		if err := os.MkdirAll(dir, 0o700); err != nil {
			return err
		}
	}
	if preferred, relocated := s.paths.Relocated(); relocated {
		s.logger.Printf("socket relocated to %s: state path is %d bytes, limit %d",
			s.paths.Socket, len(preferred), maxSocketPath)
	}
	// A stale socket from a crashed daemon must not block a fresh one.
	if err := s.clearStaleSocket(); err != nil {
		return err
	}
	unixLn, err := unixTransport{socket: s.paths.Socket}.Listen()
	if err != nil {
		return err
	}
	// Loopback unless told otherwise. Binding wider is a deliberate act:
	// the API is unauthenticated, so the network it is on is the access
	// control, and that has to be somebody's decision rather than a default.
	// The authorisation vocabulary (#1968, internal/api principal.go) now
	// names who is calling on each surface and what each operation would
	// require, and records the decision per request -- permissively: every
	// principal resolves to the owner and every check answers yes, so this
	// remains true until a policy issue says otherwise.
	host := s.Address
	if host == "" {
		host = s.set.String("daemon.address")
	}
	tcpLn, err := tcpTransport{address: net.JoinHostPort(host, strconv.Itoa(tcpPort))}.Listen()
	if err != nil {
		unixLn.Close()
		return err
	}
	s.unixLn = newConnListener(unixLn, &s.conns, nil)
	s.tcpLn = newConnListener(tcpLn, &s.conns, nil)
	// An identity listener, when one is named: the same API on the
	// daemon's tailnet, reachable by name from another network. A
	// failure here is fatal to Listen — a daemon that was asked for a
	// tsnet route and silently does not have one would be found out by
	// the first remote caller, not by its operator.
	if err := s.listenTSNet(); err != nil {
		unixLn.Close()
		tcpLn.Close()
		return err
	}
	s.endpoint = "http://" + tcpLn.Addr().String()
	s.started = time.Now()
	return s.publish()
}

// listenTSNet starts the daemon's tsnet identity listener when
// daemon.tsnet.hostname names one, and records it on the server. It is
// shared by Listen, by a takeover successor (which must recreate the
// listener: the node is in-process state and cannot be passed), and by
// a handover that did not commit (which must give the route back).
func (s *Server) listenTSNet() error {
	hostname := s.set.String("daemon.tsnet.hostname")
	if hostname == "" {
		return nil
	}
	tsLn, err := tsnetTransport{
		hostname: hostname,
		stateDir: s.tsnetStateDir(),
		authKey:  envTSNetAuthKey(),
	}.Listen()
	if err != nil {
		return err
	}
	s.tsnetLn = newConnListener(tsLn, &s.conns, nil)
	return nil
}

// tsnetStateDir is where this daemon's tsnet node keeps its identity
// (its node keys) across restarts. It is keyed by the daemon's
// configuration identity -- the same config file-set fingerprint that
// keys the socket and info file in Paths.ForConfig. Paths.ForConfig
// keys Socket and Info but leaves State shared, so without this key
// two daemons serving different configurations out of one state
// directory would share one node-state directory: the second daemon
// would load the first daemon's node keys, tsnet would ignore its
// auth key (persisted state wins), and both would compete for one
// tailnet identity. tsnet requires a unique state directory per
// service. The key is the file set, not the contents, so it is stable
// across a takeover: the successor runs the same configuration and
// lands on the same directory, keeping the identity. A daemon with no
// configuration keeps the unkeyed directory -- it is the only daemon
// its state directory can hold.
func (s *Server) tsnetStateDir() string {
	dir := filepath.Join(s.paths.State, "tsnet")
	if cfg := s.currentConfig(); cfg != nil {
		if key := FingerprintConfig(cfg.Sources); key != "" {
			dir = filepath.Join(dir, key)
		}
	}
	return dir
}

// publish writes the daemon record clients find the daemon by.
func (s *Server) publish() error {
	cfgPath := ""
	daemonKey := ""
	if cfg := s.currentConfig(); cfg != nil {
		cfgPath = cfg.Path
		daemonKey = FingerprintConfig(cfg.Sources)
	}
	return s.paths.WriteInfo(Info{
		PID:        os.Getpid(),
		Socket:     s.paths.Socket,
		Endpoint:   s.endpoint,
		ConfigPath: cfgPath,
		SchemaHash: s.reg.ConfigHash(),
		DaemonKey:  daemonKey,
		Version:    s.version,
		StartedAt:  s.started.Format(time.RFC3339),
	})
}

// startHTTP serves the API on the daemon's listeners. It is called again when
// a handoff that did not commit restores them.
func (s *Server) startHTTP() {
	mux := http.NewServeMux()
	s.routes(mux)
	srv := &http.Server{
		Handler:           s.refuseBrowserPages(s.trackActivity(s.authorize(s.requireBearer(mux)))),
		ReadHeaderTimeout: s.set.Duration("http.readHeaderTimeout"),
		ConnState:         s.observeConn,
		ConnContext:       connPrincipal,
	}
	s.httpSrv = srv
	listeners := []net.Listener{s.unixLn, s.tcpLn}
	if s.tsnetLn != nil {
		listeners = append(listeners, s.tsnetLn)
	}
	for _, ln := range listeners {
		s.httpServing.Add(1)
		go func() {
			err := srv.Serve(ln)
			// Done before the send: a test or a handoff that is not reading
			// httpErr must not leave the wait in drainHTTP hanging.
			s.httpServing.Done()
			s.httpErr <- err
		}()
	}
}

func (s *Server) clearStaleSocket() error {
	if _, err := os.Stat(s.paths.Socket); err != nil {
		return nil
	}
	c, err := net.DialTimeout("unix", s.paths.Socket, s.set.Duration("daemon.socketProbeTimeout"))
	if err == nil {
		c.Close()
		return fmt.Errorf("a daemon is already listening on %s", s.paths.Socket)
	}
	s.logger.Printf("removing stale socket %s", s.paths.Socket)
	return os.Remove(s.paths.Socket)
}

// Serve runs until the context is cancelled or a signal arrives.
// ServeInline runs the daemon's API inside the calling process, on a private
// socket, until the context ends.
//
// This is the last rung of the connection ladder: no separate process at all.
// It serves the identical handler the real daemon serves, so inline mode
// cannot drift from daemon mode -- there is only one API, hosted in two
// places.
//
// What changes is lifetime. Servers start when first called and die when the
// calling process exits, so nothing is pooled across invocations and a
// stateful server -- a browser -- cannot outlive one command. That is the
// price of not having a daemon, and it is paid on every run.
func (s *Server) ServeInline(ctx context.Context, socket string) error {
	_ = os.Remove(socket)
	ln, err := net.Listen("unix", socket)
	if err != nil {
		return err
	}
	s.unixLn = ln
	s.httpSrv = &http.Server{Handler: s.Handler(),
		ReadHeaderTimeout: s.set.Duration("http.readHeaderTimeout"),
		ConnContext:       connPrincipal}
	go func() {
		<-ctx.Done()
		s.stopServing()
		_ = os.Remove(socket)
	}()
	go func() { _ = s.httpSrv.Serve(ln) }()
	return nil
}

// Handler is the daemon's API as a plain http.Handler: the same handler
// its listeners serve, ServeInline included. One API, hosted wherever
// the caller can carry it — a host that invokes the handler directly
// (the CLI's inline mode) serves the identical API without a listener.
func (s *Server) Handler() http.Handler {
	mux := http.NewServeMux()
	s.routes(mux)
	return s.refuseBrowserPages(s.authorize(s.requireBearer(mux)))
}

// InProcessHandler returns Handler with the daemon's lifetime tied to
// ctx, the way ServeInline ties it to its socket's context: when ctx
// ends, the pools drain and the caches flush (stopServing). The caller
// owns carrying the requests; this owns only the lifetime.
func (s *Server) InProcessHandler(ctx context.Context) http.Handler {
	go func() {
		<-ctx.Done()
		s.stopServing()
	}()
	// The callers are the process itself by construction — including the
	// CLI's inline mode, whose net.Pipe transport would otherwise read
	// as a TCP caller with address "pipe" — so the principal is stamped
	// here, not inferred from the request.
	return withPrincipal(api.InProcessPrincipal(), s.Handler())
}

// ServeSocket serves the daemon's API on a unix socket until ctx ends.
//
// It is ServeInline's listener without the lifetime: ending ctx closes
// the socket and removes its file, but the pools keep running —
// InProcessHandler owns those. Inline mode opens a socket lazily, only
// when a script asks for one: a script runs in a child process, which
// can reach the in-process daemon no other way.
func (s *Server) ServeSocket(ctx context.Context, socket string) error {
	_ = os.Remove(socket)
	ln, err := net.Listen("unix", socket)
	if err != nil {
		return err
	}
	srv := &http.Server{Handler: s.Handler(),
		ReadHeaderTimeout: s.set.Duration("http.readHeaderTimeout"),
		ConnContext:       connPrincipal}
	go func() {
		<-ctx.Done()
		_ = srv.Close()
		_ = os.Remove(socket)
	}()
	go func() { _ = srv.Serve(ln) }()
	return nil
}

func (s *Server) Serve(ctx context.Context) error {
	if s.httpSrv == nil {
		s.startHTTP()
	}

	s.logger.Printf("mcpx %s listening on %s and %s", s.version, s.paths.Socket, s.endpoint)
	if cfg := s.currentConfig(); cfg != nil && cfg.Path != "" {
		s.logger.Printf("config: %s (%d servers)", cfg.Path, len(cfg.MCPServers))
	} else {
		s.logger.Printf("no config file found; run `mcpx init` to create one")
	}

	reapT := time.NewTicker(s.set.Duration("daemon.reapInterval"))
	defer reapT.Stop()
	saveT := time.NewTicker(s.set.Duration("daemon.saveInterval"))
	defer saveT.Stop()

	sigCh := make(chan os.Signal, 1)
	signal.Notify(sigCh, syscall.SIGINT, syscall.SIGTERM, syscall.SIGHUP)
	defer signal.Stop(sigCh)

	done := ctx.Done()
	for {
		select {
		case <-done:
			if s.committed.Load() {
				done = nil
				continue
			}
			return s.shutdown()
		case sig := <-sigCh:
			if s.committed.Load() {
				s.logger.Printf("received %s during takeover; ignoring it", sig)
				continue
			}
			if sig == syscall.SIGHUP {
				s.logger.Printf("received SIGHUP, reloading configuration")
				s.reloadMu.Lock()
				added, removed, err := s.reloadConfig()
				s.reloadMu.Unlock()
				if err != nil {
					s.logger.Printf("SIGHUP reload error: %v", err)
				} else {
					s.logger.Printf("SIGHUP reload successful: %d added, %d removed", len(added), len(removed))
				}
				continue
			}
			s.logger.Printf("received %s, shutting down", sig)
			return s.shutdown()
		case err := <-s.httpErr:
			if err != nil && !errors.Is(err, http.ErrServerClosed) {
				_ = s.shutdown()
				return err
			}
		case <-s.handedOff:
			// This daemon's own orphan terminations are not part of the
			// handoff it just made, so nobody else will finish them.
			s.waitOrphanTerminations()
			return nil
		case <-reapT.C:
			if s.committed.Load() {
				continue
			}
			s.watchConfig()
			s.reg.Reap()
			if s.shouldIdleExit() {
				s.logger.Printf("no activity for %s and no live instances; exiting", s.idleExit)
				return s.shutdown()
			}
		case <-saveT.C:
			if err := s.reg.SaveCache(); err != nil {
				s.logger.Printf("save cache: %v", err)
			}
			if err := s.reg.SaveSessions(); err != nil {
				s.logger.Printf("save sessions: %v", err)
			}
		}
	}
}

// trackActivity records the time of the last request so the idle-exit timer
// only fires on a genuinely unused daemon.
func (s *Server) trackActivity(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		s.lastReq.Store(time.Now().UnixNano())
		// Checked here so that an edit is live on the *next command*, not on
		// the next reap tick. Two stats and a string compare; the read only
		// happens when something moved.
		s.watchConfig()
		next.ServeHTTP(w, r)
	})
}

func (s *Server) shouldIdleExit() bool {
	if s.idleExit <= 0 || s.committed.Load() {
		return false
	}
	if time.Since(time.Unix(0, s.lastReq.Load())) < s.idleExit {
		return false
	}
	for _, st := range s.reg.Status() {
		if st.Live > 0 {
			return false
		}
	}
	return true
}

// shutdownOrphanWait bounds how long an orderly exit waits for background
// orphan-session terminations. The terminations run in parallel and each
// DELETE is already capped by UpstreamSessionDeleteTimeout, so the bound
// only fires if a termination outlives its own deadline; it is a variable
// so tests can shrink it.
var shutdownOrphanWait = defaults.UpstreamSessionDeleteTimeout

// waitOrphanTerminations blocks until the orphan-session terminations of
// this daemon's committed takeover have finished, or shutdownOrphanWait
// expires — in which case it logs and lets the exit proceed rather than
// blocking shutdown indefinitely (#298). A daemon that never took over (or
// whose takeover aborted) has no adoption and returns at once.
func (s *Server) waitOrphanTerminations() {
	ad := s.adoption.Load()
	if ad == nil {
		return
	}
	done := make(chan struct{})
	go func() {
		ad.WaitOrphanTerminations()
		close(done)
	}()
	select {
	case <-done:
	case <-time.After(shutdownOrphanWait):
		s.logger.Printf("orphan upstream session terminations still in flight after %s; exiting anyway", shutdownOrphanWait)
	}
}

func (s *Server) shutdown() error {
	// End this daemon's orphaned upstream sessions before anything else
	// goes away: the DELETEs are this daemon's own work from its takeover,
	// and process exit would kill them in flight (#298).
	s.waitOrphanTerminations()
	// Flush the schema cache before the listeners go away. A client learns the
	// daemon has stopped by watching the socket stop answering, so anything
	// persisted after that point races with whatever the client does next --
	// including removing the state directory.
	if err := s.reg.SaveCache(); err != nil {
		s.logger.Printf("save cache: %v", err)
	}
	if err := s.reg.SaveSessions(); err != nil {
		s.logger.Printf("save sessions: %v", err)
	}

	s.stopServing()
	if s.tokenStore != nil {
		_ = s.tokenStore.Close()
	}
	_ = os.Remove(s.paths.Socket)
	s.paths.RemoveInfo()
	s.logger.Printf("stopped")
	return nil
}

// stopServing stops the front door and then the pools. The HTTP server goes
// first, so calls already running can finish and take the leases they still
// need. The pools are then drained with what is left of the drain window, so
// the whole stop is bounded by pool.ExecDrainTimeout, the same window a reload
// gives a replaced pool. The HTTP grace is capped to that window too, so a
// long http.shutdownGrace cannot stretch the stop past it.
func (s *Server) stopServing() {
	window := pool.ExecDrainTimeout
	start := time.Now()
	if s.httpSrv != nil {
		grace := defaults.HTTPShutdownGrace
		if s.set != nil {
			grace = s.set.Duration("http.shutdownGrace")
		}
		ctx, cancel := context.WithTimeout(context.Background(), min(grace, window))
		_ = s.httpSrv.Shutdown(ctx)
		cancel()
	}
	if s.reg != nil {
		s.reg.CloseWithin(window - time.Since(start))
	}
}

// WarmAsync fetches schemas in the background so the first real call is fast
// without delaying the listener.
func (s *Server) WarmAsync() {
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(),
			s.set.Duration("daemon.warmTimeout"))
		defer cancel()
		start := time.Now()
		errs := s.reg.Warm(ctx, false)
		s.logger.Printf("warm complete in %s (%d failed)", time.Since(start).Truncate(time.Millisecond), len(errs))
	}()
}

// ---- routes ----

func (s *Server) routes(mux *http.ServeMux) {
	s.route(mux, "GET /v1/health", s.handleHealth)
	s.route(mux, "POST "+takeoverRoute, s.handleTakeover)
	s.route(mux, "POST "+upgradeRoute, s.handleUpgrade)
	s.route(mux, "GET /v1/status", s.handleStatus)
	// Server-sent events, because they are plain HTTP: they go through every
	// proxy that HTTP goes through, reconnect themselves in a browser, and
	// need nothing but a GET to consume. A WebSocket would buy
	// bidirectionality nobody needs here -- answers go back as ordinary
	// POSTs -- at the cost of an upgrade that half of all middleboxes
	// mishandle.
	s.route(mux, "GET /v1/events", s.handleEvents)
	// Elicitation over the daemon's own API, so a client that already holds
	// the socket -- the plugin, the TUI -- can see and answer questions
	// without spawning the binary to do it. The 135x difference between a
	// socket call and a spawn is exactly the difference that matters for
	// something sitting on the path of every tool call.
	s.route(mux, "GET /v1/elicit", s.handleElicitList)
	s.route(mux, "GET /v1/elicit/{id}", s.handleElicitGet)
	s.route(mux, "POST /v1/elicit/{id}/{action}", s.handleElicitAnswer)
	s.route(mux, "POST /v1/log", s.handleLogRecord)
	s.route(mux, "GET /v1/resource-templates", func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, http.StatusOK, map[string]any{
			"resourceTemplates": s.reg.ResourceTemplates(splitCSV(r.URL.Query().Get("ns"))),
		})
	})
	s.route(mux, "GET /v1/prompts", func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, http.StatusOK, map[string]any{
			"prompts": s.reg.Prompts(splitCSV(r.URL.Query().Get("ns"))),
		})
	})
	s.route(mux, "GET /v1/resources", func(w http.ResponseWriter, r *http.Request) {
		writeJSON(w, http.StatusOK, map[string]any{
			"resources": s.reg.Resources(splitCSV(r.URL.Query().Get("ns"))),
		})
	})
	s.route(mux, "POST /v1/prompt", func(w http.ResponseWriter, r *http.Request) {
		var req struct {
			Server    string            `json:"server"`
			Name      string            `json:"name"`
			Arguments map[string]string `json:"arguments"`
			SessionID string            `json:"sessionId"`
			CallID    string            `json:"callId"`
		}
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			writeJSON(w, http.StatusBadRequest, map[string]any{"error": err.Error()})
			return
		}
		cc := config.CallContext{SessionID: req.SessionID, CallID: req.CallID}
		out, err := s.reg.GetPrompt(r.Context(), req.Server, req.Name, req.Arguments, cc)
		if err != nil {
			writeJSON(w, failureStatus(err), map[string]any{"error": err.Error()})
			return
		}
		writeJSON(w, http.StatusOK, map[string]any{"result": out})
	})
	s.route(mux, "GET /v1/namespaces", s.handleNamespaces)
	s.route(mux, "GET /v1/tools", s.handleTools)
	s.route(mux, "GET /v1/search", s.handleSearch)
	s.route(mux, "GET /v1/types", s.handleTypes)
	s.route(mux, "GET /v1/catalog", s.handleCatalog)
	s.route(mux, "GET /v1/client.ts", s.handleClient)
	s.route(mux, "GET /v1/globals.d.ts", s.handleGlobals)
	s.route(mux, "POST /v1/call", s.handleCall)
	s.route(mux, "POST /v1/batch", s.handleBatch)
	s.route(mux, "POST /v1/resource", s.handleResource)
	s.route(mux, "POST /v1/session/release", s.handleRelease)
	s.route(mux, "POST /v1/refresh", s.handleRefresh)
	s.route(mux, "POST /v1/reload", s.handleReload)
	s.route(mux, "POST /v1/restart", s.handleRestart)
	s.route(mux, "GET /dashboard", s.handleDashboard)
	s.route(mux, "GET /v1/dashboard", s.handleDashboard)
	s.route(mux, "GET /dashboard/", s.handleDashboardAsset)
	s.route(mux, "GET /v1/dashboard/", s.handleDashboardAsset)
	s.route(mux, "GET /v1/metrics/tokens", s.handleMetricsTokens)
	s.route(mux, "GET /v1/metrics/tools", s.handleMetricsTools)
	s.route(mux, "POST /v1/chat/completions", s.handleChatCompletions)
	s.route(mux, "POST /v1/embeddings", s.handleEmbeddings)
	s.route(mux, "GET /v1/embeddings/export", s.handleEmbeddingsExport)
	s.route(mux, "POST /v1/embeddings/import", s.handleEmbeddingsImport)
	s.route(mux, "POST /v1/feedback", s.handleFeedbackSubmit)
	s.route(mux, "GET /v1/interactions", s.handleInteractionsList)
	s.route(mux, "GET /v1/interactions/{id}", s.handleInteractionGet)
	s.route(mux, "POST /v1/daemon/restart", s.handleDaemonRestart)
	s.route(mux, "POST /v1/shutdown", s.handleShutdown)
	s.routesExec(mux)
	s.routesSettings(mux)
	s.routesSecrets(mux)
	// Everything declared in internal/api that is not above. A parity test
	// fails if the two ever disagree.
	s.routesV1Ops(mux)
	s.routesProto(mux)
	// mcpx's own MCP surface, on the listeners the daemon already has.
	// Two HTTP servers with overlapping /v1 prefixes was one owner too
	// many: whichever you reached decided which half of the API existed.
	if s.MCP != nil {
		path := s.MCPPath
		if path == "" {
			path = defaults.ProtoMCPPath
		}
		s.route(mux, path, s.MCP.ServeHTTP)
	}
	s.routesConsumer(mux)
	s.routesResolve(mux)
}

func writeJSON(w http.ResponseWriter, code int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(code)
	_ = json.NewEncoder(w).Encode(v)
}

func writeErr(w http.ResponseWriter, code int, err error) {
	writeJSON(w, code, map[string]string{"error": err.Error()})
}

func writeText(w http.ResponseWriter, code int, s string) {
	w.Header().Set("Content-Type", "text/plain; charset=utf-8")
	w.WriteHeader(code)
	_, _ = w.Write([]byte(s))
}

func (s *Server) handleHealth(w http.ResponseWriter, _ *http.Request) {
	writeJSON(w, 200, map[string]any{
		"status":   "ok",
		"version":  s.version,
		"pid":      os.Getpid(),
		"uptime":   time.Since(s.started).Truncate(time.Second).String(),
		"endpoint": s.endpoint,
	})
}

func (s *Server) handleStatus(w http.ResponseWriter, _ *http.Request) {
	cfgPath := ""
	if cfg := s.currentConfig(); cfg != nil {
		cfgPath = cfg.Path
	}
	writeJSON(w, 200, map[string]any{
		"version":  s.version,
		"pid":      os.Getpid(),
		"uptime":   time.Since(s.started).Truncate(time.Second).String(),
		"endpoint": s.endpoint,
		"socket":   s.paths.Socket,
		"config":   cfgPath,
		"servers":  s.reg.Status(),
	})
}

// profileOf reads the profile selection from a request's query string.
func profileOf(r *http.Request) config.Profile {
	q := r.URL.Query()
	return config.Profile{
		Names:       splitWords(q.Get("profile")),
		SkipDefault: q.Get("skipDefault") == "1",
		All:         q.Get("allProfiles") == "1",
	}
}

func (s *Server) handleNamespaces(w http.ResponseWriter, r *http.Request) {
	writeJSON(w, 200, s.reg.Namespaces(profileOf(r)))
}

func (s *Server) handleTools(w http.ResponseWriter, r *http.Request) {
	ns := splitCSV(r.URL.Query().Get("ns"))
	writeJSON(w, 200, s.reg.Tools(ns))
}

func (s *Server) handleSearch(w http.ResponseWriter, r *http.Request) {
	limit, _ := strconv.Atoi(r.URL.Query().Get("limit"))
	if limit <= 0 {
		cs := s.callSettings(r)
		if cs != nil {
			limit = cs.Int("search.limit")
		} else {
			limit = defaults.SearchLimit
		}
	}
	semStr := r.URL.Query().Get("semantic")
	semantic := semStr == "true" || semStr == "1"
	if !semantic {
		cs := s.callSettings(r)
		if cs != nil {
			semantic = cs.Bool("search.semantic")
		}
	}
	tag := r.URL.Query().Get("tag")
	explain := r.URL.Query().Get("explain") == "true" || r.URL.Query().Get("explain") == "1"
	if tag != "" || explain {
		writeJSON(w, 200, s.reg.SearchSemanticOpts(r.URL.Query().Get("q"), limit, semantic, tag, explain))
		return
	}
	writeJSON(w, 200, s.reg.SearchSemantic(r.URL.Query().Get("q"), limit, semantic))
}

func (s *Server) handleTypes(w http.ResponseWriter, r *http.Request) {
	nss, err := s.reg.CodegenNamespaces(splitCSV(r.URL.Query().Get("ns")), profileOf(r))
	if err != nil {
		writeErr(w, 400, err)
		return
	}
	if r.URL.Query().Get("instructions") == "0" {
		for i := range nss {
			nss[i].Instructions = ""
		}
	}
	writeText(w, 200, codegen.Declarations(nss))
}

func (s *Server) handleCatalog(w http.ResponseWriter, r *http.Request) {
	nss, err := s.reg.CodegenNamespaces(splitCSV(r.URL.Query().Get("ns")), profileOf(r))
	if err != nil {
		writeErr(w, 400, err)
		return
	}
	cs := s.callSettings(r)
	budget, _ := strconv.Atoi(r.URL.Query().Get("budget"))
	if budget <= 0 {
		budget = cs.Int("catalog.budget")
	}
	bias := splitWords(r.URL.Query().Get("bias"))
	if len(bias) == 0 {
		bias = cs.List("catalog.bias")
	}
	writeText(w, 200, codegen.Catalog(nss, codegen.CatalogOptions{
		Budget: budget,
		Bias:   bias,
	}))
}

func splitWords(s string) []string {
	return strings.Fields(strings.ReplaceAll(s, ",", " "))
}

func (s *Server) handleClient(w http.ResponseWriter, r *http.Request) {
	nss, err := s.reg.CodegenNamespaces(splitCSV(r.URL.Query().Get("ns")), profileOf(r))
	if err != nil {
		writeErr(w, 400, err)
		return
	}
	session := r.URL.Query().Get("session")
	w.Header().Set("Content-Type", "application/typescript")
	_, _ = w.Write([]byte(codegen.Module(nss, s.endpoint, session)))
}

func (s *Server) handleGlobals(w http.ResponseWriter, r *http.Request) {
	nss, err := s.reg.CodegenNamespaces(splitCSV(r.URL.Query().Get("ns")), profileOf(r))
	if err != nil {
		writeErr(w, 400, err)
		return
	}
	w.Header().Set("Content-Type", "application/typescript")
	_, _ = w.Write([]byte(codegen.GlobalDeclarations(nss)))
}

type callReq struct {
	Server  string             `json:"server"`
	Tool    string             `json:"tool"`
	Args    json.RawMessage    `json:"args"`
	Context config.CallContext `json:"context"`
	// Session is the shorthand a caller may send instead of a full context.
	Session string `json:"session"`
	// Task, when present, asks for a handle now and the result later.
	Task *struct {
		TTL int64 `json:"ttl"`
	} `json:"task"`
	// Relay, when present, asks for the upstream's progress and log
	// messages during the call; see CallRelay.
	Relay *CallRelay `json:"relay,omitempty"`
}

// callContext merges the JSON body with the header shorthands, so a plain
// curl can still reach a scoped server without constructing a context object.
func callContext(r *http.Request, body config.CallContext, session string) config.CallContext {
	cc := body
	if cc.SessionID == "" {
		cc.SessionID = firstNonEmpty(session, r.Header.Get("X-Mcpx-Session"))
	}
	if cc.CallID == "" {
		cc.CallID = firstNonEmpty(r.Header.Get("X-Mcpx-Call"), cc.SessionID)
	}
	// With nothing to name the caller, the scope fallback would put every
	// such request in one shared "call:anonymous" instance -- the accidental
	// sharing config/scope.go promises it avoids. A fresh id per request is
	// the per-call isolation it promises instead; the lease reaper releases it.
	if cc.CallID == "" {
		cc.CallID = string(logging.NewTraceID("anon"))
	}
	if cc.ParentSessionID == "" {
		cc.ParentSessionID = r.Header.Get("X-Mcpx-Parent-Session")
	}
	if cc.Cwd == "" {
		cc.Cwd = r.Header.Get("X-Mcpx-Cwd")
	}
	if cc.PID == 0 {
		if n, err := strconv.Atoi(r.Header.Get("X-Mcpx-Pid")); err == nil {
			cc.PID = n
		}
	}
	// A caller reporting this daemon's own PID is reporting a process
	// every caller of the daemon shares -- /v1/exec runs, whose script
	// env carries the daemon's os.Getpid(). That PID keys shared
	// instances; it must never read as caller-private when an ephemeral
	// run ends (#1942 follow-up).
	if cc.PID > 0 && cc.PID == os.Getpid() {
		cc.PIDShared = true
	}
	if !cc.Ephemeral {
		cc.Ephemeral = r.Header.Get("X-Mcpx-Ephemeral") == "1"
	}
	return cc
}

func (s *Server) handleCall(w http.ResponseWriter, r *http.Request) {
	var req callReq
	if err := json.NewDecoder(http.MaxBytesReader(w, r.Body, s.set.Bytes("http.callBodyLimit"))).Decode(&req); err != nil {
		writeErr(w, 400, err)
		return
	}
	if req.Server == "" || req.Tool == "" {
		writeErr(w, 400, errors.New("server and tool are required"))
		return
	}
	cc := callContext(r, req.Context, req.Session)

	var args any = map[string]any{}
	if len(req.Args) > 0 {
		if err := json.Unmarshal(req.Args, &args); err != nil {
			writeErr(w, 400, fmt.Errorf("args: %w", err))
			return
		}
	}

	// A caller that asked for a task gets a handle immediately and collects
	// from /v1/tasks, rather than holding this request open for however long
	// the tool takes.
	if req.Task != nil {
		// The task counts against the caller's session, else its run
		// (as taskRequestor keys /v1/tasks), else the unbound bucket.
		requestor := tasks.Caller(cc.SessionID)
		if cc.SessionID == "" {
			requestor = tasks.Caller(r.Header.Get("X-Mcpx-Run"))
		}
		task, terr := s.startCallTask(req.Task.TTL, req.Server, req.Tool, requestor, cc, args)
		if terr != nil {
			// The store's per-requestor cap (protoTasks.maxPerRequestor).
			writeErr(w, http.StatusTooManyRequests, terr)
			return
		}
		writeJSON(w, http.StatusAccepted, map[string]any{"task": task})
		return
	}

	if r.Header.Get(InputHeader) == InputReport && s.reg.broker != nil {
		s.handleCallReporting(w, r, req, cc, args)
		return
	}

	ctx := withRun(withCallSettings(withClientCaps(r.Context(), r), s.callSettings(r)), r.Header.Get("X-Mcpx-Run"))
	reply := writeJSON
	if req.Relay != nil {
		rw := &relayWriter{w: w}
		ctx = mcpclient.WithRelay(ctx, rw.relay(req.Relay))
		reply = func(_ http.ResponseWriter, code int, v any) { rw.finish(code, v) }
	}
	start := time.Now()
	res, err := s.reg.Call(ctx, req.Server, req.Tool, cc, args)
	dur := time.Since(start).Truncate(time.Millisecond)
	s.reportCallFrame(ctx, req.Server, req.Tool, args, start, res, err)

	toolKey := req.Server + ":" + req.Tool
	var inBytes, outBytes []byte
	inBytes, _ = json.Marshal(args)
	if err == nil {
		outBytes, _ = json.Marshal(res)
	}

	if fb := s.reg.FeedbackStore(); fb != nil {
		_ = fb.RecordInteraction(logstore.InteractionRecord{
			TraceID:   cc.CallID,
			Timestamp: start,
			SessionID: cc.SessionID,
			Source:    "call",
			Input:     string(inBytes),
			Output:    string(outBytes),
			ToolsUsed: []string{toolKey},
		})
	}

	if err != nil {
		s.logger.Printf("call %s.%s failed in %s: %v", req.Server, req.Tool, dur, err)
		reply(w, failureStatus(err), failureBody(err))
		return
	}
	reply(w, 200, map[string]any{"result": res, "durationMs": dur.Milliseconds(), "traceId": cc.CallID})
}

type batchCallReqItem struct {
	Server    string          `json:"server"`
	Namespace string          `json:"namespace"`
	Tool      string          `json:"tool"`
	Name      string          `json:"name"`
	Args      json.RawMessage `json:"args"`
	Arguments json.RawMessage `json:"arguments"`
}

type batchReq struct {
	Calls       []batchCallReqItem `json:"calls"`
	StopOnError *bool              `json:"stopOnError"`
	Parallel    bool               `json:"parallel"`
	Context     config.CallContext `json:"context"`
	Session     string             `json:"session"`
}

func (s *Server) handleBatch(w http.ResponseWriter, r *http.Request) {
	var req batchReq
	limit := defaults.HTTPCallBodyLimit
	if s.set != nil {
		limit = s.set.Bytes("http.callBodyLimit")
	}
	if err := json.NewDecoder(http.MaxBytesReader(w, r.Body, limit)).Decode(&req); err != nil {
		writeErr(w, 400, err)
		return
	}
	if len(req.Calls) == 0 {
		writeJSON(w, 200, map[string]any{"results": []any{}})
		return
	}

	stopOnErr := true
	if req.StopOnError != nil {
		stopOnErr = *req.StopOnError
	}

	cc := callContext(r, req.Context, req.Session)
	ctx := withRun(withCallSettings(withClientCaps(r.Context(), r), s.callSettings(r)), r.Header.Get("X-Mcpx-Run"))

	results := make([]map[string]any, len(req.Calls))

	runOne := func(ctx context.Context, i int, item batchCallReqItem) (map[string]any, error) {
		server := item.Server
		if server == "" {
			server = item.Namespace
		}
		tool := item.Tool
		if tool == "" {
			tool = item.Name
		}
		if server == "" && strings.Contains(tool, ".") {
			server, tool, _ = strings.Cut(tool, ".")
		}
		if server == "" || tool == "" {
			return map[string]any{
				"index":   i,
				"server":  server,
				"tool":    tool,
				"status":  "error",
				"error":   "server and tool are required",
				"isError": true,
			}, errors.New("server and tool are required")
		}

		rawArgs := item.Args
		if len(rawArgs) == 0 {
			rawArgs = item.Arguments
		}
		var args any = map[string]any{}
		if len(rawArgs) > 0 {
			if err := json.Unmarshal(rawArgs, &args); err != nil {
				return map[string]any{
					"index":   i,
					"server":  server,
					"tool":    tool,
					"status":  "error",
					"error":   fmt.Sprintf("args: %v", err),
					"isError": true,
				}, err
			}
		}

		start := time.Now()
		res, err := s.reg.Call(ctx, server, tool, cc, args)
		dur := time.Since(start).Truncate(time.Millisecond)
		// Batch items bypass the single-call handlers, so report each
		// completed one here: a script's batch(...) calls belong in
		// its exec stream like any other call.
		s.reportCallFrame(ctx, server, tool, args, start, res, err)

		if err != nil {
			return map[string]any{
				"index":      i,
				"server":     server,
				"tool":       tool,
				"status":     "error",
				"error":      err.Error(),
				"isError":    true,
				"durationMs": dur.Milliseconds(),
			}, err
		}

		return map[string]any{
			"index":      i,
			"server":     server,
			"tool":       tool,
			"status":     "success",
			"result":     res,
			"durationMs": dur.Milliseconds(),
		}, nil
	}

	if req.Parallel {
		var wg sync.WaitGroup
		wg.Add(len(req.Calls))
		for i, item := range req.Calls {
			go func(i int, item batchCallReqItem) {
				defer wg.Done()
				res, _ := runOne(ctx, i, item)
				results[i] = res
			}(i, item)
		}
		wg.Wait()
	} else {
		for i, item := range req.Calls {
			res, err := runOne(ctx, i, item)
			results[i] = res
			if stopOnErr && err != nil {
				results = results[:i+1]
				break
			}
		}
	}

	writeJSON(w, 200, map[string]any{"results": results})
}

type resourceReq struct {
	Server  string             `json:"server"`
	URI     string             `json:"uri"`
	Context config.CallContext `json:"context"`
	Session string             `json:"session"`
}

func (s *Server) handleResource(w http.ResponseWriter, r *http.Request) {
	var req resourceReq
	if err := json.NewDecoder(http.MaxBytesReader(w, r.Body, s.set.Bytes("http.bodyLimit"))).Decode(&req); err != nil {
		writeErr(w, 400, err)
		return
	}
	cc := callContext(r, req.Context, req.Session)
	res, err := s.reg.ReadResource(r.Context(), req.Server, req.URI, cc)
	if err != nil {
		writeErr(w, failureStatus(err), err)
		return
	}
	writeJSON(w, 200, map[string]any{"result": res})
}

func (s *Server) handleRelease(w http.ResponseWriter, r *http.Request) {
	var req struct {
		Session string `json:"session"`
	}
	_ = json.NewDecoder(http.MaxBytesReader(w, r.Body, s.set.Bytes("http.controlBodyLimit"))).Decode(&req)
	if req.Session == "" {
		req.Session = r.Header.Get("X-Mcpx-Session")
	}
	writeJSON(w, 200, map[string]any{"released": s.reg.ReleaseCaller(req.Session)})
}

func (s *Server) handleRefresh(w http.ResponseWriter, r *http.Request) {
	// The configuration is re-read first. Refreshing schemas while still
	// serving a server list from a file read hours ago answered half the
	// question and made the other half look broken: an entry added by hand,
	// or by `mcpx registry add`, stayed invisible until the daemon was
	// restarted.
	added, removed, rerr := s.reloadConfig()
	if rerr != nil {
		s.logger.Printf("reload during refresh: %v", rerr)
	}
	ctx, cancel := context.WithTimeout(r.Context(), s.set.Duration("daemon.refreshTimeout"))
	defer cancel()
	errs := s.reg.Warm(ctx, true)
	out := map[string]string{}
	for k, v := range errs {
		out[k] = v.Error()
	}
	writeJSON(w, 200, map[string]any{
		"namespaces": s.reg.Namespaces(config.Profile{All: true}),
		"added":      added, "removed": removed, "errors": out})
}

func (s *Server) handleReload(w http.ResponseWriter, r *http.Request) {
	s.reloadMu.Lock()
	added, removed, err := s.reloadConfig()
	s.reloadMu.Unlock()
	if err != nil {
		s.logger.Printf("HTTP reload error: %v", err)
		writeErr(w, http.StatusInternalServerError, err)
		return
	}
	s.logger.Printf("HTTP reload successful: %d added, %d removed", len(added), len(removed))
	writeJSON(w, 200, map[string]any{
		"status":     "reloaded",
		"added":      added,
		"removed":    removed,
		"namespaces": s.reg.Namespaces(config.Profile{All: true}),
	})
}

func (s *Server) handleRestart(w http.ResponseWriter, r *http.Request) {
	var req struct {
		Server string `json:"server"`
		Lazy   bool   `json:"lazy"`
	}
	_ = json.NewDecoder(http.MaxBytesReader(w, r.Body, s.set.Bytes("http.controlBodyLimit"))).Decode(&req)
	res, err := s.reg.Restart(r.Context(), req.Server, req.Lazy)
	if err != nil {
		writeErr(w, 400, err)
		return
	}
	stopped, started, failed := 0, 0, 0
	for _, x := range res {
		stopped += x.Stopped
		started += len(x.Started)
		failed += len(x.Failed)
	}
	// 200 even when a replacement failed: the restart ran and the body says
	// what came back. The CLI turns failed > 0 into a non-zero exit.
	writeJSON(w, 200, map[string]any{
		"stopped": stopped, "started": started, "failed": failed,
		"lazy": req.Lazy, "servers": res})
}

func (s *Server) handleShutdown(w http.ResponseWriter, _ *http.Request) {
	writeJSON(w, 200, map[string]any{"status": "stopping"})
	go func() {
		time.Sleep(defaults.DaemonRestartSettle)
		p, _ := os.FindProcess(os.Getpid())
		_ = p.Signal(syscall.SIGTERM)
	}()
}

// execSelf is replaced in tests: a real exec re-runs the test binary.
var execSelf = syscall.Exec

func (s *Server) handleDaemonRestart(w http.ResponseWriter, _ *http.Request) {
	writeJSON(w, 200, map[string]any{"status": "restarting"})
	go func() {
		time.Sleep(defaults.DaemonRestartSettle)
		// syscall.Exec replaces this process image instantly, so a
		// background orphan-session DELETE from this daemon's takeover
		// would die in flight and leak the session upstream. Wait for
		// the terminations (bounded) before re-executing (#301).
		s.waitOrphanTerminations()
		if s.reg != nil {
			if err := s.reg.SaveCache(); err != nil && s.logger != nil {
				s.logger.Printf("save cache before restart: %v", err)
			}
		}
		execPath, err := os.Executable()
		if err == nil {
			s.stopServing()
			if s.paths.Socket != "" {
				_ = os.Remove(s.paths.Socket)
			}
			if s.paths.Info != "" {
				s.paths.RemoveInfo()
			}
			if s.logger != nil {
				s.logger.Printf("re-executing daemon: %s", execPath)
			}
			if execErr := execSelf(execPath, os.Args, os.Environ()); execErr != nil && s.logger != nil {
				s.logger.Printf("re-exec failed: %v", execErr)
			}
		}
		p, _ := os.FindProcess(os.Getpid())
		_ = p.Signal(syscall.SIGTERM)
	}()
}

func splitCSV(s string) []string {
	if strings.TrimSpace(s) == "" {
		return nil
	}
	parts := strings.Split(s, ",")
	out := parts[:0]
	for _, p := range parts {
		if p = strings.TrimSpace(p); p != "" {
			out = append(out, p)
		}
	}
	return out
}

// handleEvents streams the event bus as server-sent events.
//
// Query parameters select what to hear:
//
//	kinds=elicit,server   prefixes; empty means everything
//	session=s1            one session's events only
//	server=github         one server's
//	uri=file:///a          resource updates for these URIs
//	since=42              replay everything after sequence 42
//
// Last-Event-ID is honoured as well as since, because that is the header a
// browser's EventSource sends on reconnect -- so a reconnecting browser
// resumes without any code knowing it disconnected.
func (s *Server) handleEvents(w http.ResponseWriter, r *http.Request) {
	flusher, ok := w.(http.Flusher)
	if !ok {
		http.Error(w, "streaming unsupported", http.StatusInternalServerError)
		return
	}
	q := r.URL.Query()
	f := events.Filter{
		Kinds:   splitCSV(q.Get("kinds")),
		Session: q.Get("session"),
		Server:  q.Get("server"),
	}
	// Naming resources subscribes to them upstream for as long as this
	// stream is open. See watchResources in routes_proto.go.
	var watch *resourceWatch
	f.URIs, watch = s.watchResources(r.Context(), f.Server, splitCSV(q.Get("uri")))
	defer watch.release()
	raw := firstNonEmptyStr(r.Header.Get("Last-Event-ID"), q.Get("since"))
	since, _ := strconv.ParseUint(raw, 10, 64)
	// Present-but-zero replays everything retained; absent is live only.
	sub, gap := s.Events.SubscribeFrom(f, since, raw != "")
	defer sub.Close()

	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-cache")
	w.Header().Set("Connection", "keep-alive")
	// Nginx buffers event streams by default, which turns a live stream
	// into one that arrives all at once when the connection closes.
	w.Header().Set("X-Accel-Buffering", "no")
	w.WriteHeader(http.StatusOK)

	fmt.Fprintf(w, "retry: %d\n", s.set.Duration("http.sseRetry").Milliseconds())
	if gap {
		// Said in-band, so a subscriber can resynchronise instead of
		// trusting a stream with a hole in it.
		fmt.Fprintf(w, "event: gap\ndata: {\"since\":%d,\"latest\":%d}\n\n", since, s.Events.Latest())
	}
	if len(f.URIs) > 0 {
		watch.announce(w)
	}
	flusher.Flush()

	// A comment every so often keeps idle proxies from deciding the
	// connection is dead. The default sits under the usual sixty.
	ping := time.NewTicker(s.set.Duration("http.ssePing"))
	defer ping.Stop()

	// lastSeq is the sequence of the last event this client actually
	// received. A live drop leaves a hole after it, and the gap notice names
	// that hole rather than the client discovering it by finding a sequence
	// number missing -- which is exactly the silent loss the bus promises
	// not to have.
	lastSeq := since
	// toldDrops is the subscription's drop count at the last notice, so one
	// stall produces one gap event and not a notice per delivered event.
	var toldDrops uint64
	tellGap := func() {
		dropped := sub.Dropped.Load()
		if dropped == 0 || dropped == toldDrops {
			return
		}
		toldDrops = dropped
		fmt.Fprintf(w, "event: gap\ndata: {\"since\":%d,\"latest\":%d,\"dropped\":%d}\n\n",
			lastSeq, s.Events.Latest(), dropped)
		flusher.Flush()
	}

	for {
		select {
		case <-r.Context().Done():
			return
		case <-ping.C:
			// Checked here too: a subscriber that stopped reading long
			// enough to lose events has nothing new to read afterwards, so
			// waiting for the next event to report the loss could wait
			// forever.
			tellGap()
			fmt.Fprintf(w, ": ping\n\n")
			flusher.Flush()
		case e, open := <-sub.C:
			if !open {
				return
			}
			tellGap()
			b, err := json.Marshal(e)
			if err != nil {
				continue
			}
			fmt.Fprintf(w, "id: %d\nevent: %s\ndata: %s\n\n", e.Seq, e.Kind, b)
			lastSeq = e.Seq
			flusher.Flush()
		}
	}
}

func firstNonEmptyStr(vals ...string) string {
	for _, v := range vals {
		if v != "" {
			return v
		}
	}
	return ""
}

func (s *Server) handleElicitList(w http.ResponseWriter, r *http.Request) {
	if s.reg.broker == nil {
		writeJSON(w, http.StatusOK, []any{})
		return
	}
	// The ceiling was a constant inside the broker, so elicit.pendingLimit
	// was a number `mcpx settings` reported and nothing enforced. It is
	// passed in rather than read there because the broker is a store and a
	// store should not be reading policy.
	pending, err := s.reg.broker.Pending(elicit.Filter{
		Session:  r.URL.Query().Get("session"),
		Audience: elicit.Audience(r.URL.Query().Get("audience")),
		Limit:    s.callSettings(r).Int("elicit.pendingLimit"),
	})
	if err != nil {
		writeJSON(w, http.StatusInternalServerError, map[string]any{"error": err.Error()})
		return
	}
	if pending == nil {
		pending = []elicit.Request{}
	}
	writeJSON(w, http.StatusOK, pending)
}

func (s *Server) handleElicitGet(w http.ResponseWriter, r *http.Request) {
	if s.reg.broker == nil {
		writeJSON(w, http.StatusNotFound, map[string]any{"error": "elicitation is disabled"})
		return
	}
	req, ok, err := s.reg.broker.Get(r.PathValue("id"))
	if err != nil || !ok {
		writeJSON(w, http.StatusNotFound, map[string]any{"error": "no elicitation " + r.PathValue("id")})
		return
	}
	out := map[string]any{"request": req}
	if ans, answered, _ := s.reg.broker.Lookup(req.ID); answered {
		out["answer"] = ans
	}
	writeJSON(w, http.StatusOK, out)
}

// handleElicitAnswer takes accept, decline or cancel as the last path
// segment, so the action is in the URL and the body is only ever content.
func (s *Server) handleElicitAnswer(w http.ResponseWriter, r *http.Request) {
	if s.reg.broker == nil {
		writeJSON(w, http.StatusNotFound, map[string]any{"error": "elicitation is disabled"})
		return
	}
	action := elicit.Action(r.PathValue("action"))
	switch action {
	case elicit.Accept, elicit.Decline, elicit.Cancel:
	default:
		writeJSON(w, http.StatusBadRequest, map[string]any{
			"error": "action is accept, decline or cancel"})
		return
	}
	var content json.RawMessage
	if action == elicit.Accept {
		b, _ := io.ReadAll(io.LimitReader(r.Body, s.set.Bytes("http.bodyLimit")))
		if len(b) > 0 {
			content = b
		}
	}
	by := r.Header.Get("X-Mcpx-Answerer")
	if by == "" {
		by = "api"
	}
	if err := s.reg.broker.Respond(elicit.Answer{
		ID: r.PathValue("id"), Action: action, Content: content, By: by,
	}); err != nil {
		writeJSON(w, http.StatusConflict, map[string]any{"error": err.Error()})
		return
	}
	s.Events.Publish(events.Event{Kind: events.ElicitAnswered,
		Data: mustJSON(map[string]any{"id": r.PathValue("id"), "action": action, "by": by})})
	writeJSON(w, http.StatusOK, map[string]any{"ok": true})
}

// handleLogRecord is `mcpx log record` over the daemon's own API.
//
// The opencode plugin records a timing after every tool call. Spawning the
// binary for that costs ~23ms a call; this costs a fraction of a
// millisecond over the socket. The body is exactly what the command takes,
// parsed by the same function, so a record cannot differ by how it arrived.
//
// The level is a query parameter rather than a body field so that it can
// never collide with an attribute the caller happens to call "level".
func (s *Server) handleLogRecord(w http.ResponseWriter, r *http.Request) {
	if s.sink == nil {
		writeJSON(w, http.StatusServiceUnavailable, map[string]any{
			"error": "the durable log is unavailable in this daemon"})
		return
	}
	b, err := io.ReadAll(io.LimitReader(r.Body, s.set.Bytes("http.bodyLimit")))
	if err != nil {
		writeJSON(w, http.StatusBadRequest, map[string]any{"error": err.Error()})
		return
	}
	rec, err := logging.ExternalRecord(b, r.URL.Query().Get("level"))
	if err != nil {
		writeJSON(w, http.StatusBadRequest, map[string]any{"error": err.Error()})
		return
	}
	s.sink.Write(rec, nil)
	writeJSON(w, http.StatusOK, map[string]any{"ok": true})
}

// callRegistry returns the daemon's shared exec call-frame registry,
// creating it on first use.
func (s *Server) callRegistry() *execsvc.CallRegistry {
	s.callOnce.Do(func() { s.calls = execsvc.NewCallRegistry() })
	return s.calls
}

// reportCallFrame tells the exec stream that is running this call, if
// any, that the call happened: a call frame with the tool's resolved
// destructiveHint. A call with no run id, or a run this daemon is not
// executing, has no stream to join and is not reported.
func (s *Server) reportCallFrame(ctx context.Context, server, tool string, args any, start time.Time, res json.RawMessage, callErr error) {
	run := runFrom(ctx)
	if run == "" {
		return
	}
	// A tool that ran and returned isError failed, even though the
	// transport call itself succeeded: the frame must say so, the way
	// the path-per-tool handler already does with resultFailed.
	if callErr == nil && resultFailed(res) {
		callErr = errors.New("tool returned an error result (isError)")
	}
	var dh *bool
	tools := s.reg.Tools([]string{server})
	// Dispatch resolves an exact server name before a namespace
	// (Registry.Pool), so the annotations must be picked the same way:
	// a server whose namespace equals another server's name must not
	// lend its annotations to that server's calls.
	var picked *ToolInfo
	for i := range tools {
		if tools[i].Tool == tool && tools[i].Server == server {
			picked = &tools[i]
			break
		}
	}
	if picked == nil {
		for i := range tools {
			if tools[i].Tool == tool && tools[i].Namespace == server {
				picked = &tools[i]
				break
			}
		}
	}
	if picked != nil {
		d := destructiveHint(picked.Annotations)
		dh = &d
	}
	s.callRegistry().ReportCall(run, server, tool, args, time.Since(start), callErr, dh)
}
