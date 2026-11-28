package mcpclient

import (
	"context"
	"sync"
	"testing"
)

// TestAnswerNegotiatedSnapshotRace guards the Codex finding on PR 2443:
// handleServerRequest runs answer in a separate goroutine while a legacy
// HTTP reinitialization (finishLegacy, after a session-ending 404) writes
// the negotiated version. answer must read the version through the atomic
// mirror, never the plain field. Run with -race: reverting answer to read
// c.Negotiated makes this fail.
func TestAnswerNegotiatedSnapshotRace(t *testing.T) {
	c := &Client{}
	versions := []string{"2025-06-18", "2025-11-25"}
	var wg sync.WaitGroup
	wg.Add(2)
	go func() {
		defer wg.Done()
		for i := 0; i < 200; i++ {
			v := versions[i%len(versions)]
			c.Negotiated = v
			c.negotiated.Store(v)
		}
	}()
	go func() {
		defer wg.Done()
		for i := 0; i < 200; i++ {
			if _, rerr := c.answer(context.Background(), "ping", nil); rerr != nil {
				t.Errorf("answer ping: %v", rerr)
				return
			}
		}
	}()
	wg.Wait()
	if got := c.negotiatedVersion(); got != "2025-06-18" && got != "2025-11-25" {
		t.Errorf("negotiatedVersion = %q, want a published version", got)
	}
}
