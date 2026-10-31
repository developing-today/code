package mcpclient

import (
	"bufio"
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"syscall"
	"time"

	"github.com/dezren39/mcpx/internal/defaults"
	"github.com/dezren39/mcpx/internal/progspec"
)

// Supervised stdio transport — Phase 1 of the mcpx supervisor design
// (#1384, issue #1385). A per-server supervisor process owns a stdio
// child's pipes for the child's whole life; daemons attach to it over a
// unix socket, one at a time, and detach without signalling the child.
// Everything here is additive: the direct NewStdio path is untouched and
// the supervised path is selected only when a server's config sets the
// supervised flag (default off).

// SupervisorProtocolVersion is the attach-handshake version. A mismatch
// is refused with an explicit error; there is no silent fallback.
const SupervisorProtocolVersion = 1

// DefaultUnattachedBuffer bounds the child output a supervisor retains
// while no daemon is attached (1 MiB, tail-keep; see tailBuffer).
const DefaultUnattachedBuffer = 1 << 20

// DefaultAttachedWriteTimeout is the default bound on a single write to
// the attached daemon (see AttachedWriteTimeout).
const DefaultAttachedWriteTimeout = 5 * time.Second

// AttachedWriteTimeout bounds every write to the attached daemon's
// connection. A daemon that stops reading must not wedge the supervisor:
// once the socket buffer fills, an unbounded write would block the
// stdout pump forever — the child exit would never surface and detach
// could never proceed (#2390). On expiry the attach is dropped, the
// unwritten bytes go to the tail buffer, and the pump keeps draining
// the child. It is a variable so tests can shorten it.
var AttachedWriteTimeout = DefaultAttachedWriteTimeout

// connWriter is the per-connection writer for the attached daemon. All
// writes to the attached connection serialise on its own mutex, never
// on the supervisor's state mutex s.mu, and every write carries the
// AttachedWriteTimeout deadline. s.mu only snapshots, publishes, and
// clears the *connWriter pointer.
type connWriter struct {
	mu   sync.Mutex
	conn net.Conn
}

// Write writes p under the writer mutex with a fresh write deadline.
func (w *connWriter) Write(p []byte) (int, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	return w.writeLocked(p)
}

func (w *connWriter) writeLocked(p []byte) (int, error) {
	_ = w.conn.SetWriteDeadline(time.Now().Add(AttachedWriteTimeout))
	return w.conn.Write(p)
}

// flushLocked writes the handshake reply and then the buffered child
// output as one serialised unit. The caller (handleAttach) holds w.mu
// across the publish of w as the attached connection and this flush, so
// no live pump chunk can overtake the reply or the buffered bytes.
func (w *connWriter) flushLocked(reply supervisorReply, buffered []byte) error {
	b, err := json.Marshal(reply)
	if err != nil {
		return err
	}
	if _, err := w.writeLocked(append(b, '\n')); err != nil {
		return err
	}
	if len(buffered) > 0 {
		_, err := w.writeLocked(buffered)
		return err
	}
	return nil
}

// Close closes the underlying connection.
func (w *connWriter) Close() { w.conn.Close() }

// closeAfterFlush closes the connection only after any in-flight
// initial flush completes (Codex P2 on PR #2861). handleAttach holds
// w.mu across the publish of w and the handshake+buffer flush, so
// acquiring w.mu here is a barrier on that flush: without it,
// finishWhenDone could observe pumpDone (child exited with no further
// stdout during the flush), see attachedEver && !reported, and close
// the connection mid-flush, losing bytes already drained from the
// tail buffer. The barrier is bounded — every flush write carries
// AttachedWriteTimeout, and at finishWhenDone's call sites the stdout
// pump has already exited, so the flush is the only holder it can
// wait on. No connection write happens under s.mu.
func (w *connWriter) closeAfterFlush() {
	w.mu.Lock()
	w.mu.Unlock()
	w.Close()
}

type supervisorHandshake struct {
	Version int `json:"version"`
}

type supervisorReply struct {
	Version      int    `json:"version,omitempty"`
	Error        string `json:"error,omitempty"`
	PID          int    `json:"pid,omitempty"`
	DroppedBytes int64  `json:"droppedBytes,omitempty"`
	ChildState   string `json:"childState,omitempty"` // "running" | "exited"
	ExitErr      string `json:"exitErr,omitempty"`
	StderrTail   string `json:"stderrTail,omitempty"`
}

// SupervisorSocketPath names the per-instance socket inside the mcpx
// state directory. The server name is hashed so any configured name is
// a safe, short file name; long state paths fall back the way the
// daemon socket does (hashed name under a private temp directory).
func SupervisorSocketPath(stateDir, name string) string {
	sum := sha256.Sum256([]byte(name))
	key := hex.EncodeToString(sum[:])[:16]
	preferred := filepath.Join(stateDir, "sup-"+key+".sock")
	if len(preferred) <= 100 {
		return preferred
	}
	raw := sha256.Sum256([]byte(stateDir + "\x00" + name))
	short := hex.EncodeToString(raw[:])[:16]
	return filepath.Join(os.TempDir(), "mcpx-sup", short+".sock")
}

// StateDir resolves the mcpx state directory from the environment the
// same way daemon.ResolvePaths does (which this package cannot import:
// daemon imports pool, and pool imports mcpclient).
func StateDir() string {
	if v := os.Getenv("MCPX_STATE_DIR"); v != "" {
		return v
	}
	if v := os.Getenv("XDG_STATE_HOME"); v != "" {
		return filepath.Join(v, "mcpx")
	}
	home, _ := os.UserHomeDir()
	return filepath.Join(home, ".local", "state", "mcpx")
}

// tailBuffer is the bounded unattached-output buffer: it keeps the tail
// up to size bytes; on overflow it drops to the next newline boundary
// so a reattaching daemon's stream starts on a frame boundary, and it
// counts every dropped byte cumulatively.
type tailBuffer struct {
	mu      sync.Mutex
	buf     []byte
	size    int
	dropped int64
}

func newTailBuffer(size int) *tailBuffer { return &tailBuffer{size: size} }

func (b *tailBuffer) Write(p []byte) {
	b.mu.Lock()
	defer b.mu.Unlock()
	b.buf = append(b.buf, p...)
	for len(b.buf) > b.size {
		excess := len(b.buf) - b.size
		cut := excess
		if i := bytes.IndexByte(b.buf[excess:], '\n'); i >= 0 {
			cut = excess + i + 1
		}
		b.dropped += int64(cut)
		b.buf = b.buf[cut:]
	}
}

// Drain returns the buffered bytes and resets the buffer. The dropped
// counter is cumulative and is not reset.
func (b *tailBuffer) Drain() []byte {
	b.mu.Lock()
	defer b.mu.Unlock()
	out := b.buf
	b.buf = nil
	return out
}

func (b *tailBuffer) Dropped() int64 {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.dropped
}

// SupervisorOptions configure one supervisor process.
type SupervisorOptions struct {
	SocketPath string
	Command    string
	Args       []string
	Env        map[string]string
	Cwd        string
	InheritEnv bool
	BufferSize int
}

// supervisor is the runtime state of RunSupervisor.
type supervisor struct {
	opts    SupervisorOptions
	child   *exec.Cmd
	stdin   *os.File
	stdoutR *os.File
	stderr  *ringBuffer
	ln      net.Listener

	mu           sync.Mutex
	attached     *connWriter
	attachedEver bool
	reported     bool
	buf          *tailBuffer
	exited       bool
	exitErr      error
	stateChanged chan struct{}

	pumpDone chan struct{}
	waitDone chan struct{}
}

// RunSupervisor runs the supervisor in the current process: it spawns
// the child with NewStdio's process semantics, serves one attached
// daemon at a time on the socket, and returns when the child has exited
// and no daemon is attached. It is what `mcpx supervise` runs.
func RunSupervisor(opts SupervisorOptions) error {
	if opts.Command == "" {
		return errors.New("supervise: empty command")
	}
	if opts.SocketPath == "" {
		return errors.New("supervise: empty socket path")
	}
	if opts.BufferSize <= 0 {
		opts.BufferSize = DefaultUnattachedBuffer
	}
	if dir := filepath.Dir(opts.SocketPath); dir != "" {
		if err := os.MkdirAll(dir, 0o700); err != nil {
			return err
		}
	}
	// A socket left by a dead supervisor is stale by definition: this
	// process is the supervisor for this child's life, and it is not
	// running yet.
	_ = os.Remove(opts.SocketPath)
	ln, err := net.Listen("unix", opts.SocketPath)
	if err != nil {
		return fmt.Errorf("supervise: listen %s: %w", opts.SocketPath, err)
	}
	defer ln.Close()
	defer os.Remove(opts.SocketPath)
	if err := os.Chmod(opts.SocketPath, os.FileMode(defaults.PrivateMode)); err != nil {
		return fmt.Errorf("supervise: chmod socket: %w", err)
	}

	s := &supervisor{
		opts:         opts,
		stderr:       newRingBuffer(64 << 10),
		buf:          newTailBuffer(opts.BufferSize),
		pumpDone:     make(chan struct{}),
		waitDone:     make(chan struct{}),
		stateChanged: make(chan struct{}, 1),
	}
	if err := s.spawnChild(); err != nil {
		return err
	}
	s.ln = ln
	go s.pumpStdout()
	go func() {
		err := s.child.Wait()
		s.mu.Lock()
		s.exitErr = err
		s.exited = true
		s.mu.Unlock()
		s.signal()
		close(s.waitDone)
	}()
	go s.finishWhenDone()

	for {
		conn, err := ln.Accept()
		if err != nil {
			// The listener is closed by finishWhenDone once the child
			// is gone and drained; that is the normal way out.
			<-s.waitDone
			return nil
		}
		s.handleAttach(conn)
	}
}

// spawnChild mirrors NewStdio's process semantics: own process group,
// plain os.Pipes, stderr into a 64 KiB ring.
func (s *supervisor) spawnChild() error {
	opts := s.opts
	// A nix: program spec resolves once, here at spawn, exactly as on
	// the direct stdio path (progspec, #1921): the child is the built
	// store path, never a nix process and never the literal spec string
	// handed to exec (which could only fail not-found).
	command := opts.Command
	if progspec.IsNix(command) {
		resolved, err := progspec.ResolvePathInDir(context.Background(), command, opts.Cwd)
		if err != nil {
			return fmt.Errorf("supervise: %w", err)
		}
		command = resolved
	}
	cmd := exec.Command(command, opts.Args...)
	cmd.Dir = opts.Cwd
	env := []string{}
	if opts.InheritEnv {
		env = append(env, os.Environ()...)
	}
	for k, v := range opts.Env {
		env = append(env, k+"="+v)
	}
	cmd.Env = env
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}

	stdinR, stdinW, err := os.Pipe()
	if err != nil {
		return err
	}
	outR, outW, err := os.Pipe()
	if err != nil {
		stdinR.Close()
		stdinW.Close()
		return err
	}
	errR, errW, err := os.Pipe()
	if err != nil {
		stdinR.Close()
		stdinW.Close()
		outR.Close()
		outW.Close()
		return err
	}
	cmd.Stdin, cmd.Stdout, cmd.Stderr = stdinR, outW, errW
	startErr := cmd.Start()
	stdinR.Close()
	outW.Close()
	errW.Close()
	if startErr != nil {
		stdinW.Close()
		outR.Close()
		errR.Close()
		return fmt.Errorf("start %s: %w", opts.Command, startErr)
	}
	s.child = cmd
	s.stdin = stdinW
	go func() {
		_, _ = io.Copy(s.stderr, errR)
		errR.Close()
	}()
	// The stdout pump reads outR; keep it on the supervisor.
	s.stdoutR = outR
	return nil
}

// pumpStdout moves child stdout to the attached daemon, or into the
// bounded tail buffer while nobody is attached. It never writes while
// holding s.mu (#2390): s.mu only snapshots the attached writer, the
// write itself serialises on the writer's own mutex under a bounded
// deadline, so a stalled daemon can block this pump for at most
// AttachedWriteTimeout — never the supervisor's state, and never
// forever.
func (s *supervisor) pumpStdout() {
	defer close(s.pumpDone)
	chunk := make([]byte, 32<<10)
	for {
		n, err := s.stdoutR.Read(chunk)
		if n > 0 {
			data := chunk[:n]
			// The unattached decision and the tail-buffer append
			// are atomic under s.mu (Codex P2 on PR #2861): if the
			// snapshot is nil, the append happens while still
			// holding s.mu, so a chunk read just before an attach
			// is either inside the buffer that attach drains or
			// routed to the published writer — never stranded in
			// the buffer behind a live stream, which would
			// withhold it and reverse stream order. buf.Write is
			// a bounded memory append, not a connection write, so
			// holding s.mu across it does not restore the #2390
			// hazard.
			s.mu.Lock()
			w := s.attached
			if w == nil {
				s.buf.Write(data)
			}
			s.mu.Unlock()
			if w != nil {
				if _, werr := w.Write(data); werr != nil {
					// The daemon went away mid-write (e.g. it was
					// SIGKILLed) or stopped reading until the
					// write deadline expired: keep the bytes it
					// missed, and drop the attach — but only if
					// it still points at this same writer, so a
					// newer attach is never cleared by a stale
					// pump error.
					s.buf.Write(data)
					s.mu.Lock()
					if s.attached == w {
						s.attached = nil
					}
					s.mu.Unlock()
					w.Close()
					s.signal()
				}
			}
		}
		if err != nil {
			return
		}
	}
}

// handleAttach runs the handshake for one connection. On success the
// connection becomes the attached daemon (buffered output first, then
// the live pump) and a stdin pump starts; detach is the connection
// going away, which never signals the child.
func (s *supervisor) handleAttach(conn net.Conn) {
	r := bufio.NewReader(conn)
	_ = conn.SetReadDeadline(time.Now().Add(10 * time.Second))
	line, err := r.ReadBytes('\n')
	if err != nil {
		conn.Close()
		return
	}
	_ = conn.SetReadDeadline(time.Time{})
	var hs supervisorHandshake
	if err := json.Unmarshal(line, &hs); err != nil {
		writeReply(conn, supervisorReply{Error: "bad handshake: " + err.Error()})
		conn.Close()
		return
	}
	if hs.Version != SupervisorProtocolVersion {
		writeReply(conn, supervisorReply{Error: fmt.Sprintf(
			"supervisor protocol version mismatch: daemon sent %d, supervisor speaks %d",
			hs.Version, SupervisorProtocolVersion)})
		conn.Close()
		return
	}

	s.mu.Lock()
	if s.attached != nil {
		s.mu.Unlock()
		writeReply(conn, supervisorReply{Error: "already attached"})
		conn.Close()
		return
	}
	reply := supervisorReply{
		Version:      SupervisorProtocolVersion,
		PID:          s.child.Process.Pid,
		DroppedBytes: s.buf.Dropped(),
		ChildState:   "running",
	}
	if s.exited {
		reply.ChildState = "exited"
		if s.exitErr != nil {
			reply.ExitErr = s.exitErr.Error()
		}
		reply.StderrTail = s.stderr.Tail(800)
	}
	buffered := s.buf.Drain()
	// Publish the writer as the attached connection BEFORE the reply
	// and buffered flush go out, and hold the writer's mutex across
	// the publish and the flush (#2390): the pump snapshots the writer
	// under s.mu but serialises its writes on w.mu, so while the flush
	// holds w.mu no live chunk can overtake the reply or the buffered
	// bytes — and s.mu itself is never held across a connection write,
	// so a stalled daemon blocks only this flush, only until the write
	// deadline.
	w := &connWriter{conn: conn}
	w.mu.Lock()
	s.attached = w
	s.attachedEver = true
	if reply.ChildState == "exited" {
		s.reported = true
	}
	s.mu.Unlock()
	werr := w.flushLocked(reply, buffered)
	w.mu.Unlock()
	if werr != nil {
		s.detach(w)
		return
	}
	if reply.ChildState == "exited" {
		// This attach was the reporting handshake: the exit has now
		// been reported once, and the connection closing is the end.
		// finishWhenDone observes exited && detached and returns.
		go func() {
			_, _ = io.Copy(io.Discard, r)
			s.detach(w)
		}()
		return
	}
	go func() {
		_, _ = io.Copy(s.stdin, r)
		// The daemon detached (clean close or death). The child's
		// stdin stays open: EOF there is a lifecycle signal a
		// supervised child must not see just because a daemon left.
		s.detach(w)
	}()
}

func (s *supervisor) detach(w *connWriter) {
	s.mu.Lock()
	if s.attached == w {
		s.attached = nil
	}
	s.mu.Unlock()
	w.Close()
	s.signal()
}

func (s *supervisor) signal() {
	select {
	case s.stateChanged <- struct{}{}:
	default:
	}
}

func writeReply(conn net.Conn, reply supervisorReply) error {
	b, err := json.Marshal(reply)
	if err != nil {
		return err
	}
	_, err = conn.Write(append(b, '\n'))
	return err
}

// finishWhenDone ends the supervisor once the child has exited and
// its remaining output has drained: an attached daemon gets EOF after
// the drain, which is how a direct child's exit surfaces today. If no
// daemon has ever attached — the common case being a child that died
// before the daemon that spawned this supervisor for it could attach —
// the supervisor waits a bounded grace for that first, reporting
// attach; with nobody attached after that there is nobody to report
// to, so it exits (no lingering supervisors for dead children).
func (s *supervisor) finishWhenDone() {
	<-s.waitDone
	<-s.pumpDone
	grace := time.NewTimer(10 * time.Second)
	defer grace.Stop()
	for {
		s.mu.Lock()
		attached := s.attached
		ever := s.attachedEver
		reported := s.reported
		s.mu.Unlock()
		if attached != nil && (reported || ever) {
			// The exit reaches the attached daemon as EOF after the
			// drain; a reporting attach ends when the daemon closes.
			if reported {
				select {
				case <-s.stateChanged:
					continue
				case <-grace.C:
				}
			}
			attached.closeAfterFlush()
			s.mu.Lock()
			if s.attached == attached {
				s.attached = nil
			}
			s.mu.Unlock()
			break
		}
		if ever || reported {
			break
		}
		// Nobody has ever attached: bounded grace for the first one.
		select {
		case <-s.stateChanged:
		case <-grace.C:
			// No daemon came; close anything that raced in and exit.
			s.mu.Lock()
			attached := s.attached
			s.mu.Unlock()
			if attached != nil {
				attached.closeAfterFlush()
			}
			if s.ln != nil {
				s.ln.Close()
			}
			return
		}
	}
	// Closing the listener wakes Accept in RunSupervisor, which then
	// returns.
	if s.ln != nil {
		s.ln.Close()
	}
}

// supervisorRefusal is a live supervisor's explicit no: the socket is
// alive, so spawning a replacement would be wrong.
type supervisorRefusal struct{ msg string }

func (e *supervisorRefusal) Error() string { return "supervisor refused attach: " + e.msg }

// SupervisedOptions configure a supervised transport.
type SupervisedOptions struct {
	// Name identifies the server; it names the supervisor socket.
	Name string
	// StateDir is the mcpx state directory (default: StateDir()).
	StateDir string
	Command  string
	Args     []string
	Env      map[string]string
	Cwd      string
	// InheritEnv includes the parent environment for the child (via
	// the supervisor process's environment).
	InheritEnv bool
	// BufferSize is the supervisor's unattached-output bound.
	BufferSize int
	// SpawnTimeout bounds waiting for a spawned supervisor's socket.
	SpawnTimeout time.Duration
	// Context, when set, bounds the fail-fast nix: program-spec check
	// before a supervisor is spawned. The supervisor itself resolves the
	// spec again when it spawns the child. Nil means context.Background.
	Context context.Context
}

// SupervisedTransport speaks to a stdio child through its supervisor.
// Close detaches; it never signals the child.
type SupervisedTransport struct {
	conn    net.Conn
	stdout  *bufio.Reader
	pending []byte

	writeMu sync.Mutex
	readMu  sync.Mutex
	closed  atomic.Bool

	pid        int
	dropped    int64
	label      string
	exited     bool
	exitErr    string
	stderrTail string
}

// NewSupervised attaches to the server's supervisor, spawning it
// (detached re-exec of this binary's `supervise` subcommand) when the
// socket is not live. A connect-refused stale socket is removed and
// the supervisor respawned. When the flag is on there is no fallback
// to a direct spawn: a supervised failure is an error.
func NewSupervised(opts SupervisedOptions) (*SupervisedTransport, error) {
	return newSupervised(opts, nil)
}

// spawnFunc starts a supervisor for the socket; nil means re-exec.
type spawnFunc func(sock string, opts SupervisedOptions) error

func newSupervised(opts SupervisedOptions, spawn spawnFunc) (*SupervisedTransport, error) {
	if opts.Command == "" {
		return nil, errors.New("supervised transport: empty command")
	}
	if opts.StateDir == "" {
		opts.StateDir = StateDir()
	}
	if opts.SpawnTimeout <= 0 {
		opts.SpawnTimeout = 5 * time.Second
	}
	sock := SupervisorSocketPath(opts.StateDir, opts.Name)

	tr, err := attachSupervisor(sock, opts)
	if err == nil {
		return tr, nil
	}
	var refusal *supervisorRefusal
	if errors.As(err, &refusal) {
		// A live supervisor answered and refused (already attached,
		// version mismatch): that is a result, not a dead socket.
		return nil, err
	}
	// Not live: remove a possible stale socket and spawn on demand.
	spawnOpts := opts
	if progspec.IsNix(opts.Command) {
		// Fail fast, in this process, when the spec cannot resolve at
		// all — and hand the resolved store path to the supervisor
		// being spawned, in place of the spec. The supervisor runs
		// with the SERVER's environment (opts.Env rides along), whose
		// PATH may exclude nix entirely, so a supervisor left to
		// resolve the spec again at child spawn can fail where this
		// process succeeded (Codex review, PR #2585). A store path is
		// immutable and self-contained, so handing it over is safe,
		// and the supervisor's own restarts of the child stop
		// depending on nix being findable. The resolution is FRESH
		// (no memo read): a memoized path names whatever an earlier
		// context resolved, which another process cannot trust.
		// This check belongs HERE,
		// not before the attach attempt above: a live supervisor with
		// a healthy child needs no build, and a daemon reattaching
		// after a restart must not be failed by a nix that has since
		// left PATH or a flake that no longer resolves.
		ctx := opts.Context
		if ctx == nil {
			ctx = context.Background()
		}
		resolved, err := progspec.ResolvePathFreshInDir(ctx, opts.Command, opts.Cwd)
		if err != nil {
			return nil, fmt.Errorf("supervised transport: %w", err)
		}
		spawnOpts.Command = resolved
	}
	_ = os.Remove(sock)
	if spawn == nil {
		spawn = spawnSupervisorProcess
	}
	if serr := spawn(sock, spawnOpts); serr != nil {
		return nil, fmt.Errorf("supervised transport for %s: spawn supervisor: %w", opts.Name, serr)
	}
	deadline := time.Now().Add(opts.SpawnTimeout)
	var lastErr error
	for time.Now().Before(deadline) {
		tr, err := attachSupervisor(sock, opts)
		if err == nil {
			return tr, nil
		}
		lastErr = err
		time.Sleep(50 * time.Millisecond)
	}
	return nil, fmt.Errorf("supervised transport for %s: supervisor did not attach within %s: %v", opts.Name, opts.SpawnTimeout, lastErr)
}

func attachSupervisor(sock string, opts SupervisedOptions) (*SupervisedTransport, error) {
	conn, err := net.DialTimeout("unix", sock, 2*time.Second)
	if err != nil {
		return nil, err
	}
	r := bufio.NewReaderSize(conn, 1<<20)
	hs, _ := json.Marshal(supervisorHandshake{Version: SupervisorProtocolVersion})
	if _, err := conn.Write(append(hs, '\n')); err != nil {
		conn.Close()
		return nil, err
	}
	_ = conn.SetReadDeadline(time.Now().Add(5 * time.Second))
	line, err := r.ReadBytes('\n')
	_ = conn.SetReadDeadline(time.Time{})
	if err != nil {
		conn.Close()
		return nil, fmt.Errorf("supervisor handshake: %w", err)
	}
	var reply supervisorReply
	if err := json.Unmarshal(line, &reply); err != nil {
		conn.Close()
		return nil, fmt.Errorf("supervisor handshake: %w", err)
	}
	if reply.Error != "" {
		conn.Close()
		return nil, &supervisorRefusal{msg: reply.Error}
	}
	if reply.Version != SupervisorProtocolVersion {
		conn.Close()
		return nil, fmt.Errorf("supervisor protocol version mismatch: supervisor sent %d, daemon speaks %d",
			reply.Version, SupervisorProtocolVersion)
	}
	tr := &SupervisedTransport{
		conn:       conn,
		stdout:     r,
		pid:        reply.PID,
		dropped:    reply.DroppedBytes,
		label:      strings.TrimSpace(opts.Command+" "+strings.Join(opts.Args, " ")) + " (supervised)",
		exited:     reply.ChildState == "exited",
		exitErr:    reply.ExitErr,
		stderrTail: reply.StderrTail,
	}
	return tr, nil
}

// spawnSupervisorProcess re-execs this binary as `mcpx supervise`,
// detached in its own session so it outlives the daemon. The child's
// environment and working directory are carried by the supervisor
// process itself, which passes them on when it spawns the child.
func spawnSupervisorProcess(sock string, opts SupervisedOptions) error {
	exe, err := os.Executable()
	if err != nil {
		return err
	}
	if dir := filepath.Dir(sock); dir != "" {
		if err := os.MkdirAll(dir, 0o700); err != nil {
			return err
		}
	}
	args := []string{"supervise", "--socket", sock, "--"}
	args = append(args, opts.Command)
	args = append(args, opts.Args...)
	cmd := exec.Command(exe, args...)
	cmd.Dir = opts.Cwd
	env := []string{}
	if opts.InheritEnv {
		env = append(env, os.Environ()...)
	}
	for k, v := range opts.Env {
		env = append(env, k+"="+v)
	}
	cmd.Env = env
	cmd.SysProcAttr = &syscall.SysProcAttr{Setsid: true}
	logPath := sock + ".log"
	if lf, lerr := os.OpenFile(logPath, os.O_CREATE|os.O_APPEND|os.O_WRONLY, os.FileMode(defaults.PrivateMode)); lerr == nil {
		cmd.Stdout = lf
		cmd.Stderr = lf
		defer lf.Close()
	}
	devnull, derr := os.OpenFile(os.DevNull, os.O_RDONLY, 0)
	if derr == nil {
		cmd.Stdin = devnull
		defer devnull.Close()
	}
	if err := cmd.Start(); err != nil {
		return err
	}
	// Reap it if it exits while this process lives; if this process is
	// killed the supervisor is reparented and keeps the child.
	go func() { _ = cmd.Wait() }()
	return nil
}

// Send writes one frame to the child through the supervisor.
func (t *SupervisedTransport) Send(ctx context.Context, msg []byte) error {
	t.writeMu.Lock()
	defer t.writeMu.Unlock()
	if t.closed.Load() {
		return errors.New("supervised transport closed")
	}
	if t.exited {
		return fmt.Errorf("write to %s: child already exited: %s (stderr: %s)", t.label, t.exitErr, t.stderrTail)
	}
	if _, err := t.conn.Write(append(msg, '\n')); err != nil {
		return fmt.Errorf("write to %s: %w", t.label, err)
	}
	return nil
}

// Recv reads one frame from the child through the supervisor, with the
// same framing and noise-skipping as StdioTransport.Recv. When the
// attach handshake already reported the child exited, frames the child
// wrote before exiting are still delivered first; the exit error
// surfaces once they are drained.
func (t *SupervisedTransport) Recv() ([]byte, error) {
	t.readMu.Lock()
	defer t.readMu.Unlock()
	for {
		if t.exited {
			// The supervisor holds a reporting connection open, so a
			// plain read would block forever once the pre-exit frames
			// are drained; bound it and then surface the exit.
			_ = t.conn.SetReadDeadline(time.Now().Add(300 * time.Millisecond))
		}
		line, err := t.readLine()
		if t.exited {
			_ = t.conn.SetReadDeadline(time.Time{})
		}
		if err != nil {
			if t.exited {
				return nil, fmt.Errorf("%s exited: %s (stderr: %s)", t.label, t.exitErr, t.stderrTail)
			}
			if errors.Is(err, io.EOF) {
				return nil, fmt.Errorf("%s exited: supervised connection closed after child exit (stderr: %s)", t.label, t.stderrTail)
			}
			return nil, err
		}
		line = trimSpace(line)
		if len(line) == 0 {
			continue
		}
		if line[0] != '{' && line[0] != '[' {
			continue
		}
		return line, nil
	}
}

// readLine is StdioTransport.readLine's logic over the socket
// connection: bytes read but not yet framed stay in pending.
func (t *SupervisedTransport) readLine() ([]byte, error) {
	for {
		if i := bytes.IndexByte(t.pending, '\n'); i >= 0 {
			line := t.pending[:i]
			t.pending = t.pending[i+1:]
			return line, nil
		}
		if len(t.pending) > maxLine {
			return nil, fmt.Errorf("frame exceeds %d bytes", maxLine)
		}
		chunk, err := t.stdout.ReadSlice('\n')
		t.pending = append(t.pending, chunk...)
		if err != nil && err != bufio.ErrBufferFull {
			return nil, err
		}
	}
}

// Close detaches from the supervisor. The child is not signalled and
// keeps running under its supervisor.
func (t *SupervisedTransport) Close() error {
	if !t.closed.CompareAndSwap(false, true) {
		return nil
	}
	return t.conn.Close()
}

// Info describes the supervised child.
func (t *SupervisedTransport) Info() string { return t.label }

// PID returns the supervised child's process id.
func (t *SupervisedTransport) PID() int { return t.pid }

// DroppedBytes is the supervisor's cumulative dropped-byte count as of
// this transport's attach.
func (t *SupervisedTransport) DroppedBytes() int64 { return t.dropped }
