package homeutil

import (
	"os"
	"strings"
	"testing"
)

// The contract probe matrix (issues #5762 / #5763), with
// os.UserHomeDir as the oracle for the set case: HOME unset and
// HOME empty both produce the named error, and a set HOME is
// returned unchanged.
func TestHomeProbeMatrix(t *testing.T) {
	orig, had := os.LookupEnv("HOME")
	t.Cleanup(func() {
		if had {
			_ = os.Setenv("HOME", orig)
		} else {
			_ = os.Unsetenv("HOME")
		}
	})

	_ = os.Unsetenv("HOME")
	if _, err := Home(); err == nil || !strings.Contains(err.Error(), "HOME is not set or is empty") {
		t.Fatalf("unset HOME: got err=%v, want the named HOME error", err)
	}

	t.Setenv("HOME", "")
	if _, err := Home(); err == nil || !strings.Contains(err.Error(), "HOME is not set or is empty") {
		t.Fatalf("empty HOME: got err=%v, want the named HOME error", err)
	}

	t.Setenv("HOME", "/home/someone")
	home, err := Home()
	if err != nil || home != "/home/someone" {
		t.Fatalf("set HOME: got (%q, %v), want (/home/someone, nil)", home, err)
	}
}

func TestMustHomePanicsNamingHOME(t *testing.T) {
	orig, had := os.LookupEnv("HOME")
	t.Cleanup(func() {
		if had {
			_ = os.Setenv("HOME", orig)
		} else {
			_ = os.Unsetenv("HOME")
		}
	})
	_ = os.Unsetenv("HOME")
	defer func() {
		r := recover()
		if r == nil {
			t.Fatal("MustHome with HOME unset did not panic")
		}
		if !strings.Contains(r.(error).Error(), "HOME is not set or is empty") {
			t.Fatalf("panic = %v, want the named HOME error", r)
		}
	}()
	_ = MustHome()
}
