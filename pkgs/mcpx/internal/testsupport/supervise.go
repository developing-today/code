package testsupport

import (
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

// SuperviseReExec handles the supervised transport re-execing the
// test binary (#9963, Phase 3 of #1384). Supervised stdio is the
// default now, so a suite whose pools dial stdio servers spawns
// supervisors by re-execing os.Executable() as
// `supervise --socket <path> -- <child>` — the same re-exec the real
// binary serves via `mcpx supervise`. A test binary is not the mcpx
// binary: without this, the re-exec would run the whole suite again
// as the "supervisor", recursively. Call it first in TestMain; it
// exits the process when the binary was re-execed as a supervisor
// and returns at once otherwise.
func SuperviseReExec() {
	if len(os.Args) < 2 || os.Args[1] != "supervise" {
		// Suite process: arm the supervisor owner watch (#10774) so
		// a test binary killed mid-run (timeout, OOM) cannot leak
		// the detached supervisors it spawned.
		mcpclient.EnableSupervisorOwnerWatch()
		return
	}
	os.Exit(runSupervise(os.Args[2:]))
}

// StopSupervisors terminates every supervised stdio child recorded in the
// test's state directory. Production supervisors intentionally outlive a
// daemon so a replacement daemon can adopt them; an isolated test environment
// has no successor, so its cleanup must explicitly reap them after stopping
// the daemon.
func StopSupervisors(stateDir string) error {
	sockets, err := filepath.Glob(filepath.Join(mcpclient.SupervisorSocketDir(stateDir), "sup-*.sock"))
	if err != nil {
		return err
	}
	for _, socket := range sockets {
		deadline := time.Now().Add(15 * time.Second)
		for {
			if _, err := os.Stat(socket); os.IsNotExist(err) {
				break
			} else if err != nil {
				return fmt.Errorf("stat supervisor socket %s: %w", socket, err)
			}

			outcomes := mcpclient.ScanSupervisorSockets(stateDir, nil, nil)
			var outcome *mcpclient.SupervisorScanOutcome
			for i := range outcomes {
				if outcomes[i].Socket == socket {
					outcome = &outcomes[i]
					break
				}
			}
			// A socket missing from the scan was removed between the stat
			// and the glob: the stat at the top of the loop settles it.
			if outcome != nil && outcome.Action == "terminate-failed" {
				return fmt.Errorf("terminate supervisor %s: %s", socket, outcome.Detail)
			}
			if time.Now().After(deadline) {
				last := "not in scan"
				if outcome != nil {
					last = outcome.Action + " " + outcome.Detail
				}
				return fmt.Errorf("supervisor %s did not stop within 15s (last scan: %s)", socket, last)
			}
			time.Sleep(100 * time.Millisecond)
		}
	}
	return nil
}

func runSupervise(args []string) int {
	var socket string
	var child []string
	for i := 0; i < len(args); i++ {
		switch {
		case args[i] == "--socket" && i+1 < len(args):
			socket = args[i+1]
			i++
		case args[i] == "--":
			child = args[i+1:]
			i = len(args)
		default:
			fmt.Fprintln(os.Stderr, "supervise: unexpected argument", args[i])
			return 2
		}
	}
	if socket == "" || len(child) == 0 {
		fmt.Fprintln(os.Stderr, "supervise: want --socket <path> -- <command> [args...]")
		return 2
	}
	if err := mcpclient.RunSupervisor(mcpclient.SupervisorOptions{
		SocketPath: socket,
		Command:    child[0],
		Args:       child[1:],
		InheritEnv: true,
	}); err != nil {
		fmt.Fprintln(os.Stderr, "supervise:", err)
		return 1
	}
	return 0
}
