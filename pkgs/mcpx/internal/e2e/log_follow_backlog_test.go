package e2e_test

import (
	"fmt"
	"os"
	"os/exec"
	"strings"
	"testing"
	"time"
)

// followBurst starts `mcpx log --follow` filtered to a burst event, writes
// twelve burst records once it is polling, and returns a reader for how many
// burst lines the follow has printed so far.
func followBurst(t *testing.T, env ...string) (printed func() int, burstDone time.Time) {
	t.Helper()
	printed, _ = followBurstOutput(t, env...)
	return printed, time.Now()
}

// followBurstOutput is followBurst that also returns the follow's output.
func followBurstOutput(t *testing.T, env ...string) (printed func() int, output func() string) {
	t.Helper()
	e := newEnv(t, oneServer)
	e.setenv(env...)
	e.run("log", "record", `{"event":"harness.warmup"}`)
	var out syncBuffer
	cmd := exec.Command(e.mcpx, "log", "--follow", "--event", "harness.burst")
	cmd.Dir = e.dir
	cmd.Env = append([]string{}, e.envVars...)
	cmd.Stdout, cmd.Stderr = &out, &out
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		_ = cmd.Process.Signal(os.Interrupt)
		done := make(chan struct{})
		go func() { _ = cmd.Wait(); close(done) }()
		select {
		case <-done:
		case <-time.After(10 * time.Second):
			_ = cmd.Process.Kill()
			<-done
		}
	})
	// Let the follow take its floor and start ticking before the burst.
	time.Sleep(time.Second)
	for i := 0; i < 12; i++ {
		e.run("log", "record", fmt.Sprintf(`{"event":"harness.burst","n":%d}`, i))
	}
	t.Cleanup(func() {
		if t.Failed() {
			t.Logf("follow output:\n%s", out.String())
		}
	})
	return func() int { return strings.Count(out.String(), "event=harness.burst ") }, out.String
}

// awaitPrinted waits up to d for the follow to have printed want lines and
// returns how many it had printed when it stopped waiting.
func awaitPrinted(printed func() int, want int, d time.Duration) int {
	deadline := time.Now().Add(d)
	for {
		if n := printed(); n >= want || time.Now().After(deadline) {
			return n
		}
		time.Sleep(50 * time.Millisecond)
	}
}

// TestFollowBacklogCapsWhatOneFollowPollEmits probes logstore.followBacklog
// (#8467): it caps how many records one poll of `mcpx log --follow` prints.
// At the default a burst of twelve records is printed whole. Capped at one,
// each poll prints only the newest record it finds, so a burst that lands
// between two polls shows as its last record and the rest are skipped.
func TestFollowBacklogCapsWhatOneFollowPollEmits(t *testing.T) {
	printed, _ := followBurst(t)
	if n := awaitPrinted(printed, 12, 3*time.Second); n != 12 {
		t.Fatalf("at the default logstore.followBacklog the follow should print the whole burst of 12, printed %d", n)
	}

	var out func() string
	printed, out = followBurstOutput(t, "MCPX_LOGSTORE_FOLLOW_BACKLOG=1")
	deadline := time.Now().Add(3 * time.Second)
	for !strings.Contains(out(), "n=11") && time.Now().Before(deadline) {
		time.Sleep(50 * time.Millisecond)
	}
	if !strings.Contains(out(), "n=11") {
		t.Fatalf("logstore.followBacklog=1 should still print the newest record of the burst:\n%s", out())
	}
	// Two more polls: nothing skipped comes back later.
	time.Sleep(time.Second)
	if n := printed(); n >= 12 {
		t.Fatalf("logstore.followBacklog=1 should cap each poll at one record, but the follow printed all %d", n)
	}
}
