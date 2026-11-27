package daemon

import (
	"context"
	"testing"
	"time"
)

// Delayed detachment: a supervisor that is still clearing the
// predecessor's EOF refuses the first attaches as already attached,
// then accepts. The takeover scan must retry through the transient
// window and adopt.
func TestAdoptSupervisedWithDetachRetryAdoptsAfterTransientRefusal(t *testing.T) {
	calls := 0
	adopt := func(context.Context) (bool, bool) {
		calls++
		if calls < 3 {
			return false, true
		}
		return true, false
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	if !adoptSupervisedWithDetachRetry(ctx, adopt) {
		t.Fatal("retry should adopt once the detach clears")
	}
	if calls != 3 {
		t.Fatalf("calls = %d, want 3 (two transient refusals then adopt)", calls)
	}
}

// Persistent refusal stays a quiet no-op and expires within the bound:
// no steal, no spin past the ~2s budget.
func TestAdoptSupervisedWithDetachRetryPersistentRefusalStaysQuiet(t *testing.T) {
	calls := 0
	adopt := func(context.Context) (bool, bool) {
		calls++
		return false, true
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	start := time.Now()
	if adoptSupervisedWithDetachRetry(ctx, adopt) {
		t.Fatal("persistent already-attached must stay a quiet no-op")
	}
	elapsed := time.Since(start)
	if elapsed > adoptDetachRetryBudget+2*time.Second {
		t.Fatalf("retry ran %v, want it bounded near %v", elapsed, adoptDetachRetryBudget)
	}
	if calls < 2 {
		t.Fatalf("calls = %d, want retries while the refusal persists", calls)
	}
}

// Any other quiet no-op (no supervisor, wrong pool state) is not
// retried: only already-attached is transient.
func TestAdoptSupervisedWithDetachRetryOtherNoOpDoesNotRetry(t *testing.T) {
	calls := 0
	adopt := func(context.Context) (bool, bool) {
		calls++
		return false, false
	}
	if adoptSupervisedWithDetachRetry(context.Background(), adopt) {
		t.Fatal("non-attach refusal must not report adoption")
	}
	if calls != 1 {
		t.Fatalf("calls = %d, want exactly 1 (no retry on other no-ops)", calls)
	}
}
