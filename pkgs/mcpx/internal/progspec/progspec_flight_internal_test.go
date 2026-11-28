package progspec

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

// stubNixInternal installs a fake nix binary on PATH whose build
// creates one executable tool and prints its store path, counting
// invocations in a file. (The external test package has its own
// stub; this one serves the in-package flight tests.)
func stubNixInternal(t *testing.T) (callsFile string) {
	t.Helper()
	dir := t.TempDir()
	storeOut := filepath.Join(dir, "store", "bbbb-fakepkg-1.0")
	bin := filepath.Join(storeOut, "bin")
	if err := os.MkdirAll(bin, 0o755); err != nil {
		t.Fatal(err)
	}
	tool := filepath.Join(bin, "vanttool")
	callsFile = filepath.Join(dir, "calls")
	stub := "#!/bin/sh\n" +
		"echo x >> \"" + callsFile + "\"\n" +
		"mkdir -p \"" + bin + "\"\n" +
		"printf '#!/bin/sh\nexit 0\n' > \"" + tool + "\"\n" +
		"chmod +x \"" + tool + "\"\n" +
		"sleep 0.2\n" +
		"echo \"" + storeOut + "\"\n"
	if err := os.WriteFile(filepath.Join(dir, "nix"), []byte(stub), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", dir+string(os.PathListSeparator)+os.Getenv("PATH"))
	return callsFile
}

func nixCallsInternal(t *testing.T, callsFile string) int {
	t.Helper()
	data, err := os.ReadFile(callsFile)
	if os.IsNotExist(err) {
		return 0
	}
	if err != nil {
		t.Fatal(err)
	}
	return strings.Count(string(data), "x")
}

// #9945: a build whose output fails the completion executable
// check (deleted mid-flight) must fail the flight for the leader
// AND every waiter — nobody is handed a dead path with a nil
// error — and nothing is memoised, so a later call retries.
func TestFlightResultFailingCompletionCheckErrorsForAllCallers(t *testing.T) {
	calls := stubNixInternal(t)

	orig := flightResultExecutable
	flightResultExecutable = func(string) bool { return false }
	defer func() { flightResultExecutable = orig }()

	const spec = "nix:nixpkgs#vanttool"
	const n = 4
	var wg sync.WaitGroup
	errs := make([]error, n)
	start := make(chan struct{})
	for i := 0; i < n; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			<-start
			_, errs[i] = Resolve(context.Background(), spec)
		}(i)
	}
	close(start)
	wg.Wait()
	for i := 0; i < n; i++ {
		if errs[i] == nil {
			t.Errorf("caller %d: got nil error, want the mid-flight deletion error", i)
			continue
		}
		if !strings.Contains(errs[i].Error(), spec) || !strings.Contains(errs[i].Error(), "no longer executable") {
			t.Errorf("caller %d: err = %v, want an error naming the spec and the dead output", i, errs[i])
		}
	}
	if got := nixCallsInternal(t, calls); got != 1 {
		t.Fatalf("nix ran %d times for %d concurrent resolves, want 1 (one flight)", got, n)
	}

	// Nothing memoised: with the check restored, a later call
	// retries the build and succeeds.
	flightResultExecutable = orig
	r, err := Resolve(context.Background(), spec)
	if err != nil {
		t.Fatalf("retry after the failed flight: %v", err)
	}
	if !isExecutable(r.Path) {
		t.Errorf("retry resolved to %q, which is not executable", r.Path)
	}
	if got := nixCallsInternal(t, calls); got != 2 {
		t.Errorf("nix ran %d times, want 2 (failed flight not memoised; the retry rebuilt)", got)
	}

	// And the successful retry IS memoised.
	if _, err := Resolve(context.Background(), spec); err != nil {
		t.Fatalf("memoised resolve: %v", err)
	}
	if got := nixCallsInternal(t, calls); got != 2 {
		t.Errorf("nix ran %d times after a successful resolve, want 2 (memo hit)", got)
	}
}

// The happy path is unchanged when the completion check passes:
// concurrent callers share one successful flight.
func TestFlightResultPassingCompletionCheckStillShares(t *testing.T) {
	calls := stubNixInternal(t)
	const spec = "nix:nixpkgs#vanttool"
	const n = 3
	var wg sync.WaitGroup
	errs := make([]error, n)
	start := make(chan struct{})
	deadline := time.Now().Add(10 * time.Second)
	_ = deadline
	for i := 0; i < n; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			<-start
			_, errs[i] = Resolve(context.Background(), spec)
		}(i)
	}
	close(start)
	wg.Wait()
	for i := 0; i < n; i++ {
		if errs[i] != nil {
			t.Errorf("caller %d: %v", i, errs[i])
		}
	}
	if got := nixCallsInternal(t, calls); got != 1 {
		t.Errorf("nix ran %d times for %d concurrent resolves, want 1 (singleflight)", got, n)
	}
}
