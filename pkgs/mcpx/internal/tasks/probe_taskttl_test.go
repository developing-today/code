package tasks

import (
	"context"
	"testing"
	"time"
)

// Probe: caller-supplied TTLs reach Start with only a <=0 -> default rule.
// ttl=1e13 ms: time.Duration(1e13)*time.Millisecond overflows int64 ns,
// wraps negative, so the AfterFunc expiry fires immediately — deleting
// the task and cancelling its context at birth.
func TestProbeTaskTTLOverflow(t *testing.T) {
	s := New()
	// Control: sane TTL, work runs 150ms, task must survive and complete.
	ctl := s.Start(60000, func(ctx context.Context) (any, *Fault) {
		select {
		case <-ctx.Done():
			return nil, &Fault{Code: -32603, Message: "ctx cancelled"}
		case <-time.After(150 * time.Millisecond):
			return "ctl-ok", nil
		}
	})
	time.Sleep(400 * time.Millisecond)
	if got, ok := s.Get(ctl.TaskID); !ok {
		t.Errorf("control task missing after 400ms")
	} else {
		t.Logf("control: status=%s", got.Status)
	}

	// Overflow TTL.
	big := s.Start(10000000000000, func(ctx context.Context) (any, *Fault) {
		select {
		case <-ctx.Done():
			return nil, &Fault{Code: -32603, Message: "ctx cancelled"}
		case <-time.After(150 * time.Millisecond):
			return "big-ok", nil
		}
	})
	time.Sleep(400 * time.Millisecond)
	got, ok := s.Get(big.TaskID)
	t.Logf("overflow ttl=1e13: present=%v status=%v", ok, func() string {
		if ok {
			return string(got.Status)
		}
		return "-"
	}())
	if !ok {
		t.Errorf("BUG REPRODUCED: ttl=1e13 task deleted before its work finished")
	}

	// Huge-but-not-overflowing TTL: 1e12 ms ~= 31.7 years of retention.
	huge := s.Start(1000000000000, func(ctx context.Context) (any, *Fault) {
		return "huge-ok", nil
	})
	time.Sleep(100 * time.Millisecond)
	if got, ok := s.Get(huge.TaskID); ok {
		t.Logf("huge ttl=1e12 accepted verbatim: stored TTL=%d ms", got.TTL)
	}
}
