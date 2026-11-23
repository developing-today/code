package mcpclient

import (
	"bufio"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"syscall"
	"time"
)

// Adopt-or-reap scan — Phase 3 of #1384 (issue #9963).
//
// On daemon start the state directory holds one supervisor socket
// per supervised server that has ever run. Sockets fall into three
// classes:
//
//   - configured: the server is in the daemon's current config and
//     supervised — the pool's adoption scan (Phase 2, #9962) attaches
//     to it; the scan here leaves it alone.
//   - orphaned: no configured supervised server maps to the socket —
//     the server left the config (or opted out of supervision), so
//     nobody will ever attach again. The scan reaps it: it asks the
//     supervisor to terminate (which kills the child) and logs the
//     reap. A supervisor that answers with a refusal — a daemon is
//     attached, or a protocol-version mismatch — is NOT reaped: it
//     is live and owned, and the refusal is logged instead.
//   - stale: the socket file remains but no supervisor listens
//     (dial refused). The file is removed and the removal logged.

// SupervisorScanOutcome is one socket's scan result.
type SupervisorScanOutcome struct {
	Socket string
	// Action is one of: configured, reaped, stale-removed, refused,
	// terminate-failed.
	Action string
	Detail string
}

// ScanSupervisorSockets performs the reap/stale half of the
// adopt-or-reap scan over stateDir and returns one outcome per
// supervisor socket found, in filename order. configured lists the
// server names the daemon expects supervisors for. logf may be nil.
func ScanSupervisorSockets(stateDir string, configured []string, logf func(string, ...any)) []SupervisorScanOutcome {
	if logf == nil {
		logf = func(string, ...any) {}
	}
	expected := map[string]string{}
	for _, name := range configured {
		expected[SupervisorSocketPath(stateDir, name)] = name
	}
	matches, _ := filepath.Glob(filepath.Join(SupervisorSocketDir(stateDir), "sup-*.sock"))
	sort.Strings(matches)
	var out []SupervisorScanOutcome
	for _, sock := range matches {
		if name, ok := expected[sock]; ok {
			out = append(out, SupervisorScanOutcome{Socket: sock, Action: "configured", Detail: name})
			continue
		}
		err := TerminateSupervisorSocket(sock)
		switch {
		case err == nil:
			logf("mcpx: reaped orphaned supervisor %s (server no longer configured as supervised)", sock)
			out = append(out, SupervisorScanOutcome{Socket: sock, Action: "reaped"})
		case IsSupervisorRefusal(err):
			logf("mcpx: orphan scan left supervisor %s in place: %v", sock, err)
			out = append(out, SupervisorScanOutcome{Socket: sock, Action: "refused", Detail: err.Error()})
		case isStaleSocketErr(err):
			_ = os.Remove(sock)
			logf("mcpx: removed stale supervisor socket %s", sock)
			out = append(out, SupervisorScanOutcome{Socket: sock, Action: "stale-removed", Detail: err.Error()})
		default:
			logf("mcpx: orphan scan could not terminate supervisor %s: %v", sock, err)
			out = append(out, SupervisorScanOutcome{Socket: sock, Action: "terminate-failed", Detail: err.Error()})
		}
	}
	return out
}

// isStaleSocketErr reports whether err means nobody is listening on
// the socket: the file is stale and safe to remove.
func isStaleSocketErr(err error) bool {
	if err == nil {
		return false
	}
	if os.IsNotExist(err) {
		return true
	}
	// ENOTSOCK: a non-socket file under a sup-*.sock name (a crashed
	// write, or a test's stand-in). Linux reports dialing one as
	// connection refused; macOS as "socket operation on non-socket".
	// Either way nobody can be listening on it.
	if errors.Is(err, syscall.ECONNREFUSED) || errors.Is(err, syscall.ENOTSOCK) {
		return true
	}
	return strings.Contains(err.Error(), "connection refused") ||
		strings.Contains(err.Error(), "no such file")
}

// TerminateSupervisorSocket asks the supervisor listening on sock to
// terminate: it kills its child and exits. It is the reap half of
// the adopt-or-reap scan. A live supervisor that refuses (a daemon
// is attached, a protocol-version mismatch) returns a
// *supervisorRefusal; a dead socket returns the dial error.
func TerminateSupervisorSocket(sock string) error {
	_, err := supervisorControl(sock, "terminate")
	return err
}

// supervisorControl sends one control handshake (restart/terminate)
// to the supervisor on sock and returns its reply. Control
// handshakes never become the attached daemon.
func supervisorControl(sock, action string) (supervisorReply, error) {
	conn, err := net.DialTimeout("unix", sock, 2*time.Second)
	if err != nil {
		return supervisorReply{}, err
	}
	defer func() { _ = conn.Close() }()
	hs, _ := json.Marshal(supervisorHandshake{Version: SupervisorProtocolVersion, Action: action})
	if _, err := conn.Write(append(hs, '\n')); err != nil {
		return supervisorReply{}, err
	}
	_ = conn.SetReadDeadline(time.Now().Add(5 * time.Second))
	line, err := bufio.NewReader(conn).ReadBytes('\n')
	if err != nil {
		return supervisorReply{}, fmt.Errorf("supervisor %s: %w", action, err)
	}
	var reply supervisorReply
	if err := json.Unmarshal(line, &reply); err != nil {
		return supervisorReply{}, fmt.Errorf("supervisor %s: %w", action, err)
	}
	if reply.Error != "" {
		return reply, &supervisorRefusal{msg: reply.Error}
	}
	if reply.Version != SupervisorProtocolVersion {
		return reply, &supervisorRefusal{msg: fmt.Sprintf(
			"supervisor protocol version mismatch: supervisor sent %d, daemon speaks %d",
			reply.Version, SupervisorProtocolVersion)}
	}
	return reply, nil
}
