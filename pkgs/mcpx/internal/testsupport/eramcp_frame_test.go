package testsupport

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os/exec"
	"strings"
	"testing"
	"time"
)

// eramcp must read any frame mcpx's own transport would send. Its stdin
// scanner once used bufio's 64 KiB default: a larger frame ended the scan,
// the fake exited without answering, and a test failed for a reason that
// had nothing to do with what it tested. 256 KiB is above that default and
// far below the transport's ceiling (defaults.StdioMaxLine).
func TestEraMCPAnswersAFrameLargerThanBufioDefault(t *testing.T) {
	bin := EraMCPBinary(t)
	cmd := exec.Command(bin)
	cmd.Env = append(cmd.Environ(), "ERAMCP_MODE=legacy")
	stdin, err := cmd.StdinPipe()
	if err != nil {
		t.Fatal(err)
	}
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = cmd.Process.Kill(); _ = cmd.Wait() })

	padding := strings.Repeat("x", 256<<10)
	frame := fmt.Sprintf(`{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"},"padding":%q}}`, padding)
	if _, err := fmt.Fprintln(stdin, frame); err != nil {
		t.Fatalf("write a %d-byte frame: %v", len(frame), err)
	}

	got := make(chan string, 1)
	go func() {
		sc := bufio.NewScanner(stdout)
		if sc.Scan() {
			got <- sc.Text()
		}
		close(got)
	}()
	select {
	case line, ok := <-got:
		if !ok {
			t.Fatalf("eramcp exited without answering a %d-byte frame", len(frame))
		}
		var reply struct {
			ID     int             `json:"id"`
			Result json.RawMessage `json:"result"`
		}
		if err := json.Unmarshal([]byte(line), &reply); err != nil || reply.ID != 1 || len(reply.Result) == 0 {
			t.Fatalf("want a result for id 1, got %q", line)
		}
	case <-time.After(30 * time.Second):
		t.Fatalf("no answer to a %d-byte frame within 30s", len(frame))
	}
}
