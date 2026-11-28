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
// per supervised server instance that has ever run — sockets are
// keyed per instance, and classified here by the server segment of
// the socket's file name, so every instance of a server classifies
// the same way. Sockets fall into three classes:
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

// globSupervisorSockets returns every supervisor socket file under
// every candidate directory for stateDir (see supervisorSocketDirs),
// deduped. Scanning all candidates keeps pre-#15407 supervisors (in
// the legacy fallback dir) and relocated ones visible to the
// adopt-or-reap scan across the upgrade.
func globSupervisorSockets(stateDir string) []string {
	seen := map[string]bool{}
	var matches []string
	for _, dir := range supervisorSocketDirs(stateDir) {
		found, _ := filepath.Glob(filepath.Join(dir, "sup-*.sock"))
		for _, m := range found {
			if !seen[m] {
				seen[m] = true
				matches = append(matches, m)
			}
		}
	}
	return matches
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
		expected[supervisorNameHash(name)] = name
	}
	matches := globSupervisorSockets(stateDir)
	sort.Strings(matches)
	var out []SupervisorScanOutcome
	for _, sock := range matches {
		if hash, ok := supervisorSocketNameHash(filepath.Base(sock)); ok {
			if name, configured := expected[hash]; configured {
				out = append(out, SupervisorScanOutcome{Socket: sock, Action: "configured", Detail: name})
				continue
			}
		}
		out = append(out, terminateScanSocket(sock,
			"reaped orphaned supervisor %s (server no longer configured as supervised)",
			"orphan scan", logf))
	}
	return out
}

// ReapSupervisors terminates every supervisor of the named servers in
// stateDir — every instance of each, primary and scoped — and returns
// one outcome per socket of those servers, with the same actions as
// the scan. Sockets of other servers are untouched and unreported. A
// supervisor that refuses the terminate (a daemon is still attached)
// is left in place and reported refused: a refusal is never evidence
// the supervisor is reapable.
//
// `mcpx stop` calls this for the stopped daemon's servers (#9963):
// supervisors are built to outlive daemon restarts so a restarted
// frontend re-adopts the same child, so only an explicit stop reaps
// them — restoring the no-orphan guarantee the direct-spawn path
// always had, whose absence forced #11343 to pause the supervised
// default.
func ReapSupervisors(stateDir string, names []string, logf func(string, ...any)) []SupervisorScanOutcome {
	if logf == nil {
		logf = func(string, ...any) {}
	}
	targets := map[string]bool{}
	for _, name := range names {
		targets[supervisorNameHash(name)] = true
	}
	matches := globSupervisorSockets(stateDir)
	sort.Strings(matches)
	var out []SupervisorScanOutcome
	for _, sock := range matches {
		hash, ok := supervisorSocketNameHash(filepath.Base(sock))
		if !ok || !targets[hash] {
			continue
		}
		out = append(out, terminateScanSocket(sock,
			"reaped supervisor %s (server stopped)",
			"stop reap", logf))
	}
	return out
}

// terminateScanSocket terminates one supervisor socket and classifies
// the result. reapFormat is the log line for a landed terminate (one
// %s verb, the socket); context names the pass in the refusal and
// failure lines.
func terminateScanSocket(sock, reapFormat, context string, logf func(string, ...any)) SupervisorScanOutcome {
	err := TerminateSupervisorSocket(sock)
	switch {
	case err == nil:
		logf("mcpx: "+reapFormat, sock)
		return SupervisorScanOutcome{Socket: sock, Action: "reaped"}
	case IsSupervisorRefusal(err):
		logf("mcpx: %s left supervisor %s in place: %v", context, sock, err)
		return SupervisorScanOutcome{Socket: sock, Action: "refused", Detail: err.Error()}
	case isStaleSocketErr(err):
		_ = os.Remove(sock)
		logf("mcpx: removed stale supervisor socket %s", sock)
		return SupervisorScanOutcome{Socket: sock, Action: "stale-removed", Detail: err.Error()}
	default:
		logf("mcpx: %s could not terminate supervisor %s: %v", context, sock, err)
		return SupervisorScanOutcome{Socket: sock, Action: "terminate-failed", Detail: err.Error()}
	}
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
