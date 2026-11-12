package tasks

import (
	"testing"
	"time"
)

// Exercise the read guard with an expired item still in the map, without
// racing the expiry timer that normally removes it first.
func TestTouchLockedCannotReviveExpiredTask(t *testing.T) {
	now := time.Now()
	expired := now.Add(-time.Second)
	st := New()
	item := &Task{
		TaskID:        "tsk-expired",
		Status:        Completed,
		LastUpdatedAt: expired.Add(-time.Second),
		TTL:           60_000,
		ExpiresAt:     &expired,
	}
	st.tasks[item.TaskID] = item
	if st.touchLocked(item, now) {
		t.Fatal("touch accepted an already-expired result")
	}
	if _, ok := st.tasks[item.TaskID]; ok {
		t.Fatal("touch left the expired result in the store")
	}
}
