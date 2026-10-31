package elicit_test

// Pins #1691: concurrent answers from two processes sharing one store file.
// Exactly one wins; every loser gets the documented already-answered error,
// never the store's UNIQUE-constraint complaint. The claim is atomic, so the
// outcome (unlike the winner's identity) is deterministic — no timing
// sensitivity, no retries to green.

import (
	"path/filepath"
	"strings"
	"sync"
	"testing"

	"github.com/dezren39/mcpx/internal/elicit"
)

func TestConcurrentAnswersOneWinnerLosersToldSo(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "e.db")

	a, err := elicit.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer a.Close()
	b, err := elicit.Open(path)
	if err != nil {
		t.Fatal(err)
	}
	defer b.Close()

	r := ask(t, a, elicit.Request{})

	const racers = 8
	start := make(chan struct{})
	errs := make([]error, racers)
	var wg sync.WaitGroup
	for i := 0; i < racers; i++ {
		wg.Add(1)
		go func(i int) {
			defer wg.Done()
			h := a
			if i%2 == 1 {
				h = b
			}
			<-start
			errs[i] = h.Respond(elicit.Answer{ID: r.ID, Action: elicit.Decline, By: "probe"})
		}(i)
	}
	close(start)
	wg.Wait()

	winners := 0
	for _, err := range errs {
		switch {
		case err == nil:
			winners++
		case strings.Contains(err.Error(), "was already"):
			// The documented loss. Anything else is the store leaking.
		default:
			t.Errorf("loser got a store error instead of the documented outcome: %v", err)
		}
	}
	if winners != 1 {
		t.Errorf("winners = %d, want exactly 1", winners)
	}
}
