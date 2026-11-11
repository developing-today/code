package testsupport

import (
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"testing"
	"time"
)

// The oracle for the #1971 remainder-item-2 sweep: a process planted
// executing from inside a registered build temp dir must be reported,
// the dir must be removed, and a process executing from anywhere else
// must not be reported. If the detector stopped recognising a planted
// leftover, these tests fail — the sweep cannot pass silently.

func plantSleeper(t *testing.T, dir string) *exec.Cmd {
	t.Helper()
	sleep, err := exec.LookPath("sleep")
	if err != nil {
		t.Skipf("no sleep binary to plant: %v", err)
	}
	planted := filepath.Join(dir, "sleep")
	raw, err := os.ReadFile(sleep)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(planted, raw, 0o755); err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command(planted, "30")
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		_ = cmd.Process.Kill()
		_, _ = cmd.Process.Wait()
	})
	// Give the kernel a moment so /proc/<pid>/exe names the planted copy.
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		if _, err := os.Readlink(filepath.Join("/proc", strconv.Itoa(cmd.Process.Pid), "exe")); err == nil {
			return cmd
		}
		time.Sleep(10 * time.Millisecond)
	}
	return cmd
}

// Every fixture builder that creates a build temp dir must register it,
// or the suite-wide guard silently misses that builder's dirs and any
// process surviving from them (the class-wide coverage rule). The
// oracle is the builder sources themselves: a new builder that creates
// a mcpx-* temp dir without registering fails here.
func TestEveryFixtureBuilderRegistersItsTempDir(t *testing.T) {
	for _, file := range []string{"build.go", "build_era.go", "build_proto.go", "build_task.go"} {
		raw, err := os.ReadFile(file)
		if err != nil {
			t.Fatal(err)
		}
		src := string(raw)
		if !strings.Contains(src, "os.MkdirTemp(\"\", \"mcpx-") {
			continue
		}
		if !strings.Contains(src, "registerBuildTempDir(dir)") {
			t.Fatalf("%s creates a mcpx-* build temp dir but never calls registerBuildTempDir(dir); the suite sweep cannot see it", file)
		}
	}
}

func TestSweepReportsAProcessExecutingFromABuildDir(t *testing.T) {
	if runtime.GOOS != "linux" {
		t.Skip("leftover detection is /proc based")
	}
	// The planted sleeper never exits, so it must still be reported —
	// but the test need not wait out the full shutdown budget for it.
	prev := leftoverGracePeriod
	leftoverGracePeriod = 600 * time.Millisecond
	t.Cleanup(func() { leftoverGracePeriod = prev })
	dir, err := os.MkdirTemp("", "mcpx-bin-")
	if err != nil {
		t.Fatal(err)
	}
	registerBuildTempDir(dir)
	t.Cleanup(func() { _ = os.RemoveAll(dir) })
	cmd := plantSleeper(t, dir)

	removed, leftover := SweepBuildTempDirs()
	found := false
	for _, p := range leftover {
		if p.PID == cmd.Process.Pid {
			found = true
		}
	}
	if !found {
		t.Fatalf("sweep did not report the planted leftover process pid=%d; leftover=%v", cmd.Process.Pid, leftover)
	}
	removedFound := false
	for _, d := range removed {
		if d == dir {
			removedFound = true
		}
	}
	if !removedFound {
		t.Fatalf("sweep did not remove the registered build dir %s; removed=%v", dir, removed)
	}
	if _, err := os.Stat(dir); !os.IsNotExist(err) {
		t.Fatalf("build dir %s still exists after sweep: %v", dir, err)
	}
}

// A process that exits inside the grace budget (orderly daemon drain)
// must not be reported: the sweep polls for exit within the budget
// before classifying a survivor as a leak.
func TestSweepDoesNotReportAProcessThatExitsWithinTheGraceBudget(t *testing.T) {
	if runtime.GOOS != "linux" {
		t.Skip("leftover detection is /proc based")
	}
	prev := leftoverGracePeriod
	leftoverGracePeriod = 3 * time.Second
	t.Cleanup(func() { leftoverGracePeriod = prev })

	dir, err := os.MkdirTemp("", "mcpx-bin-")
	if err != nil {
		t.Fatal(err)
	}
	registerBuildTempDir(dir)
	t.Cleanup(func() { _ = os.RemoveAll(dir) })

	sleep, err := exec.LookPath("sleep")
	if err != nil {
		t.Skipf("no sleep binary to plant: %v", err)
	}
	planted := filepath.Join(dir, "sleep")
	raw, err := os.ReadFile(sleep)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(planted, raw, 0o755); err != nil {
		t.Fatal(err)
	}
	cmd := exec.Command(planted, "0.3")
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _, _ = cmd.Process.Wait() })

	_, leftover := SweepBuildTempDirs()
	for _, p := range leftover {
		if p.PID == cmd.Process.Pid {
			t.Fatalf("sweep reported a process that exited within the grace budget: %v", p)
		}
	}
}

func TestSweepIgnoresProcessesExecutingFromElsewhere(t *testing.T) {
	if runtime.GOOS != "linux" {
		t.Skip("leftover detection is /proc based")
	}
	dir, err := os.MkdirTemp("", "mcpx-fake-")
	if err != nil {
		t.Fatal(err)
	}
	registerBuildTempDir(dir)
	t.Cleanup(func() { _ = os.RemoveAll(dir) })

	sleep, err := exec.LookPath("sleep")
	if err != nil {
		t.Skipf("no sleep binary: %v", err)
	}
	cmd := exec.Command(sleep, "30")
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		_ = cmd.Process.Kill()
		_, _ = cmd.Process.Wait()
	})

	_, leftover := SweepBuildTempDirs()
	for _, p := range leftover {
		if p.PID == cmd.Process.Pid {
			t.Fatalf("sweep reported a process executing from outside the build dirs: %v", p)
		}
	}
}

func TestSweepWithNoLeftoversRemovesDirsAndReportsNone(t *testing.T) {
	dir, err := os.MkdirTemp("", "mcpx-bin-")
	if err != nil {
		t.Fatal(err)
	}
	registerBuildTempDir(dir)
	t.Cleanup(func() { _ = os.RemoveAll(dir) })

	removed, leftover := SweepBuildTempDirs()
	if len(leftover) != 0 {
		t.Fatalf("sweep reported leftovers where none were planted: %v", leftover)
	}
	found := false
	for _, d := range removed {
		if d == dir {
			found = true
		}
	}
	if !found {
		t.Fatalf("sweep did not remove the registered build dir %s; removed=%v", dir, removed)
	}
}
