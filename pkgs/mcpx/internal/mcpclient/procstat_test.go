package mcpclient

import (
	"os"
	"runtime"
	"testing"
)

// TestProcStatSelf checks procStat against this test process.
// Oracle: the kernel's own record of the running test process — it
// must exist, have a non-zero start time, agree with
// ProcessStartTime, and not be a zombie.
func TestProcStatSelf(t *testing.T) {
	if runtime.GOOS != "linux" && runtime.GOOS != "darwin" {
		t.Skipf("procStat has no implementation on %s (linux/darwin only, #7063)", runtime.GOOS)
	}
	pid := os.Getpid()
	start, state, err := procStat(pid)
	if err != nil {
		t.Fatalf("procStat(self): %v", err)
	}
	if start == 0 {
		t.Fatal("procStat(self): start time is zero")
	}
	if state == 'Z' {
		t.Fatal("procStat(self): running test process reported as zombie")
	}
	again, err := ProcessStartTime(pid)
	if err != nil {
		t.Fatalf("ProcessStartTime(self): %v", err)
	}
	if again != start {
		t.Fatalf("start time not stable: procStat %d, ProcessStartTime %d", start, again)
	}
	if _, _, err := procStat(-1); err == nil {
		t.Fatal("procStat(-1): expected an error for a nonexistent pid")
	}
}
