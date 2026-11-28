package daemon

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"net/url"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
)

// SSH transport (#1880): the daemon's HTTP API tunnelled over the
// operator's own ssh, so a remote daemon is reachable with the access
// control they already have -- their ssh keys, their ssh_config, their
// bastion hosts -- and mcpx needs no new credential, no listening port
// and no new Go dependency. The client runs the system `ssh` binary and
// speaks HTTP to a helper on the far side that pipes the bytes into the
// remote daemon's unix socket.
//
// This is deliberately not an embedded SSH implementation
// (golang.org/x/crypto/ssh): that would duplicate the operator's
// ssh_config, agent and known_hosts handling, and would be a second
// place for a host key policy to be wrong.

// SSHDefaultBin is the mcpx executable the remote ssh command runs. A
// remote mcpx somewhere else is named with ?bin=.
const SSHDefaultBin = "mcpx"

// SSHProxyCommand is the subcommand run on the remote host: it connects
// its stdin/stdout to that host's daemon socket. It is a public command
// (mcpx stdio-proxy) rather than a hidden one: it appears in the ssh
// command line the client actually runs, so anybody reading an error, a
// process list or `ssh host ps` has to be able to look it up, and it is
// independently useful for wiring a tunnel by hand.
const SSHProxyCommand = "stdio-proxy"

// SSHEndpoint is a parsed ssh:// daemon endpoint.
type SSHEndpoint struct {
	User string // empty means ssh's own default for the host
	Host string
	Port int    // 0 means ssh's own default (22, or whatever ssh_config says)
	Bin  string // remote mcpx executable; SSHDefaultBin unless ?bin= says otherwise
	Sock string // remote daemon socket; ?sock=, or empty for the remote process's own default
}

// String rebuilds the endpoint in the form a user typed, so errors and
// status output show the route they named rather than an internal
// http URL invented for it.
func (e SSHEndpoint) String() string {
	var b strings.Builder
	b.WriteString("ssh://")
	if e.User != "" {
		b.WriteString(e.User)
		b.WriteByte('@')
	}
	b.WriteString(e.Host)
	if e.Port != 0 {
		b.WriteByte(':')
		b.WriteString(strconv.Itoa(e.Port))
	}
	q := url.Values{}
	if e.Bin != "" && e.Bin != SSHDefaultBin {
		q.Set("bin", e.Bin)
	}
	if e.Sock != "" {
		q.Set("sock", e.Sock)
	}
	if enc := q.Encode(); enc != "" {
		b.WriteByte('?')
		b.WriteString(enc)
	}
	return b.String()
}

// Target is the destination ssh connects to.
func (e SSHEndpoint) Target() string {
	if e.User == "" {
		return e.Host
	}
	return e.User + "@" + e.Host
}

// RemoteCommand is the argv the remote ssh session runs. The socket path
// is normally resolved by the remote process itself, so no assumption
// about the remote filesystem crosses the tunnel; ?sock= overrides it.
func (e SSHEndpoint) RemoteCommand() []string {
	bin := e.Bin
	if bin == "" {
		bin = SSHDefaultBin
	}
	argv := []string{bin, SSHProxyCommand}
	if e.Sock != "" {
		argv = append(argv, "--sock", e.Sock)
	}
	return argv
}

// shellQuote renders one argument safe to embed in the remote command
// line. ssh sends the remote command to the far side's shell as a single
// string (its arguments joined with spaces), so a ?bin= or ?sock= value
// containing spaces or metacharacters would otherwise be split — or
// worse, interpreted — remotely. Arguments made only of shell-inert
// characters pass through unquoted, keeping the ordinary command line
// exactly what a reader expects to see.
func shellQuote(s string) string {
	if s == "" {
		return "''"
	}
	if strings.IndexFunc(s, func(r rune) bool {
		switch {
		case r >= 'a' && r <= 'z', r >= 'A' && r <= 'Z', r >= '0' && r <= '9':
			return false
		}
		switch r {
		case '_', '-', '.', '/', ':', '@', '%', '+', '=', ',':
			return false
		}
		return true
	}) < 0 {
		return s
	}
	return "'" + strings.ReplaceAll(s, "'", `'\''`) + "'"
}

// remoteCommandLine is RemoteCommand as the one string ssh actually
// delivers to the remote shell, each argument shell-quoted.
func (e SSHEndpoint) remoteCommandLine() string {
	argv := e.RemoteCommand()
	quoted := make([]string, len(argv))
	for i, a := range argv {
		quoted[i] = shellQuote(a)
	}
	return strings.Join(quoted, " ")
}

// ParseSSHEndpoint parses ssh://[user@]host[:port][?bin=...&sock=...].
//
// ok is false when the endpoint is not an ssh one at all; a malformed ssh
// endpoint is an error and never a silent fallback to plain TCP, because
// the fallback would send an unauthenticated API to whatever answers on
// that host's port 22.
func ParseSSHEndpoint(endpoint string) (ep SSHEndpoint, ok bool, err error) {
	rest, isSSH := strings.CutPrefix(endpoint, "ssh://")
	if !isSSH {
		return SSHEndpoint{}, false, nil
	}
	u, perr := url.Parse("ssh://" + rest)
	if perr != nil || u.Hostname() == "" {
		return SSHEndpoint{}, true, fmt.Errorf("invalid ssh endpoint %q: want ssh://[user@]host[:port][?bin=/path/to/mcpx][&sock=/path/to/daemon.sock]", endpoint)
	}
	if u.Path != "" && u.Path != "/" {
		return SSHEndpoint{}, true, fmt.Errorf("invalid ssh endpoint %q: %q is not part of an ssh endpoint (a remote socket is ?sock=)", endpoint, u.Path)
	}
	ep = SSHEndpoint{Host: u.Hostname(), Bin: SSHDefaultBin}
	if u.User != nil {
		ep.User = u.User.Username()
		if _, hasPW := u.User.Password(); hasPW {
			return SSHEndpoint{}, true, fmt.Errorf("invalid ssh endpoint %q: ssh:// carries no password; ssh's own keys, agent and ssh_config authenticate", endpoint)
		}
	}
	if p := u.Port(); p != "" {
		n, cerr := strconv.Atoi(p)
		if cerr != nil || n < 1 || n > 65535 {
			return SSHEndpoint{}, true, fmt.Errorf("invalid ssh endpoint %q: bad port %q", endpoint, p)
		}
		ep.Port = n
	}
	q := u.Query()
	for k := range q {
		switch k {
		case "bin", "sock":
		default:
			return SSHEndpoint{}, true, fmt.Errorf("invalid ssh endpoint %q: unknown parameter %q (want bin, sock)", endpoint, k)
		}
	}
	if b := q.Get("bin"); b != "" {
		ep.Bin = b
	}
	ep.Sock = q.Get("sock")
	return ep, true, nil
}

// SSHDialer dials an ssh:// endpoint by running the system ssh binary.
// It exists per endpoint and is safe for concurrent use: every call
// starts its own ssh process.
type SSHDialer struct {
	Endpoint SSHEndpoint
	// SSHBin is the local ssh executable; empty means "ssh" from PATH.
	SSHBin string
	// Stderr, when set, also receives ssh's stderr: warnings (host key
	// changes, agent messages) that are not failures. A failed
	// connection's stderr is captured and put in the error regardless.
	Stderr io.Writer
}

// NewSSHDialer builds a dialer for one endpoint.
func NewSSHDialer(ep SSHEndpoint) *SSHDialer { return &SSHDialer{Endpoint: ep} }

// Address is the placeholder host the client's HTTP URLs use. The dialer
// ignores it: ssh decides where the bytes go.
func (d *SSHDialer) Address() string { return net.JoinHostPort("ssh", "0") }

func (d *SSHDialer) sshBin() string {
	if d.SSHBin != "" {
		return d.SSHBin
	}
	return "ssh"
}

// DialContext starts ssh and returns the connection on its stdio.
//
// The process is not tied to ctx: an HTTP keep-alive connection outlives
// the dial that opened it, and killing ssh when the dialing request
// finished would close a healthy connection under the next request. The
// transport closes the conn when it is done with it, and Close kills ssh.
func (d *SSHDialer) DialContext(ctx context.Context, _, _ string) (net.Conn, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	args := []string{"-T", "-o", "BatchMode=yes"}
	if d.Endpoint.Port != 0 {
		args = append(args, "-p", strconv.Itoa(d.Endpoint.Port))
	}
	args = append(args, "--", d.Endpoint.Target())
	remote := d.Endpoint.RemoteCommand()
	// One argument, shell-quoted: ssh joins the command arguments with
	// spaces and the remote shell parses the result, so quoting here is
	// what preserves each value's argument boundary on the far side.
	args = append(args, d.Endpoint.remoteCommandLine())

	cmd := exec.Command(d.sshBin(), args...)
	// Bound how long Wait may spend on the stderr copy after ssh dies:
	// a ProxyCommand child of ssh can inherit the stderr pipe and hold
	// it open long after ssh itself is gone, and an unbounded Wait
	// would make Close (called synchronously by net/http on cancel)
	// block for that child's whole lifetime.
	cmd.WaitDelay = defaults.TransportSSHWaitDelay
	inR, inW, err := os.Pipe()
	if err != nil {
		return nil, fmt.Errorf("ssh %s: %w", d.Endpoint.Target(), err)
	}
	outR, outW, err := os.Pipe()
	if err != nil {
		_ = inR.Close()
		_ = inW.Close()
		return nil, fmt.Errorf("ssh %s: %w", d.Endpoint.Target(), err)
	}
	stderr := &cappedBuffer{max: defaults.ExecStderrLimit}
	if d.Stderr != nil {
		stderr.tee = d.Stderr
	}
	cmd.Stdin = inR
	cmd.Stdout = outW
	cmd.Stderr = stderr
	if err := cmd.Start(); err != nil {
		_ = inR.Close()
		_ = inW.Close()
		_ = outR.Close()
		_ = outW.Close()
		return nil, fmt.Errorf("ssh %s: %w", d.Endpoint.Target(), err)
	}
	// The child holds the far ends; closing our copies of them is what
	// lets Read see EOF when ssh exits.
	_ = inR.Close()
	_ = outW.Close()

	return &sshConn{
		cmd:    cmd,
		in:     inW,
		out:    outR,
		stderr: stderr,
		remote: sshAddr(d.Endpoint.Host + ":" + strconv.Itoa(d.Endpoint.Port)),
		argv:   remote,
	}, nil
}

// sshConn is one ssh process seen as a net.Conn: the HTTP bytes travel
// through its stdin and stdout.
type sshConn struct {
	cmd    *exec.Cmd
	in     io.WriteCloser // our write end of ssh's stdin
	out    io.ReadCloser  // our read end of ssh's stdout
	stderr *cappedBuffer
	remote net.Addr
	argv   []string

	closeOnce sync.Once
	waitOnce  sync.Once
	waitErr   error
	closed    bool
}

func (c *sshConn) Read(b []byte) (int, error) {
	n, err := c.out.Read(b)
	if errors.Is(err, io.EOF) {
		c.wait()
		if c.waitErr != nil {
			return n, c.failure()
		}
	}
	return n, err
}

func (c *sshConn) Write(b []byte) (int, error) {
	n, err := c.in.Write(b)
	if err != nil {
		c.wait()
		if c.waitErr != nil {
			return n, c.failure()
		}
	}
	return n, err
}

// Close shuts the tunnel down. Killing ssh rather than only closing its
// pipes matters for a cancelled request: the far end may be mid-response
// and would otherwise hold the remote socket open.
func (c *sshConn) Close() error {
	c.closeOnce.Do(func() {
		c.closed = true
		_ = c.in.Close()
		_ = c.out.Close()
		if c.cmd.Process != nil {
			_ = c.cmd.Process.Kill()
		}
		c.wait()
	})
	return nil
}

func (c *sshConn) wait() {
	c.waitOnce.Do(func() {
		c.waitErr = c.cmd.Wait()
		if c.closed && c.waitErr != nil {
			// A kill we asked for is not a failure to report — and
			// neither is the wait-delay expiry that kill can cause
			// when a proxy child still holds the stderr pipe.
			var ee *exec.ExitError
			if errors.As(c.waitErr, &ee) || errors.Is(c.waitErr, exec.ErrWaitDelay) {
				c.waitErr = nil
			}
		}
	})
}

// failure turns ssh's exit into an error that names the host and the
// remote command, because "EOF" from a tunnel says nothing about which
// of the two ends failed.
func (c *sshConn) failure() error {
	msg := strings.TrimSpace(c.stderr.String())
	if msg == "" {
		msg = c.waitErr.Error()
	}
	return fmt.Errorf("ssh %s: running %q failed: %s",
		c.remote.String(), strings.Join(c.argv, " "), msg)
}

func (c *sshConn) LocalAddr() net.Addr  { return sshAddr("local") }
func (c *sshConn) RemoteAddr() net.Addr { return c.remote }

// Deadlines are no-ops: an ssh pipe has no deadline to set. The
// request's context is what bounds a call, and the transport closes the
// connection -- which kills ssh -- when that context ends.
func (c *sshConn) SetDeadline(time.Time) error      { return nil }
func (c *sshConn) SetReadDeadline(time.Time) error  { return nil }
func (c *sshConn) SetWriteDeadline(time.Time) error { return nil }

type sshAddr string

func (a sshAddr) Network() string { return "ssh" }
func (a sshAddr) String() string  { return string(a) }

// cappedBuffer keeps the head of ssh's stderr: enough to say why it
// failed, bounded so a chatty or hostile remote cannot exhaust memory.
type cappedBuffer struct {
	mu  sync.Mutex
	buf bytes.Buffer
	max int
	tee io.Writer
}

func (b *cappedBuffer) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.tee != nil {
		_, _ = b.tee.Write(p)
	}
	if room := b.max - b.buf.Len(); room > 0 {
		if len(p) > room {
			b.buf.Write(p[:room])
		} else {
			b.buf.Write(p)
		}
	}
	return len(p), nil
}

func (b *cappedBuffer) String() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.String()
}
