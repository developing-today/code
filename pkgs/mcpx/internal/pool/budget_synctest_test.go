package pool

import (
	"context"
	"testing"
	"testing/synctest"
	"time"
)

// The #8242 evaluation's smallest first step, pool side: the #2639
// synctest pilot extended from elicit to the call budget. budget's
// clock is a time.Timer goroutine, so inside a synctest bubble it
// runs on virtual time — a question can hold a call open for ten
// virtual seconds against a 50ms budget, a span no wall-clock test
// can afford, and the pause/resume outcome is exact on every run.
func TestBudgetSynctestClockStopsWhileQuestionPending(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		var q questions
		ctx, cancel := budget(context.Background(), 50*time.Millisecond, &q)
		defer cancel()
		done := q.asking()

		// Ten virtual seconds pass with the question still pending:
		// two hundred budgets' worth. The clock is stopped, so the
		// budget must not have run out.
		time.Sleep(10 * time.Second)
		select {
		case <-ctx.Done():
			t.Fatal("the budget ran out while a question was pending")
		default:
		}

		// The question closes; the remaining budget resumes and the
		// call times out with the budget's own cause.
		done()
		<-ctx.Done()
		if cause := context.Cause(ctx); cause != errCallTimeout {
			t.Fatalf("cause = %v, want %v", cause, errCallTimeout)
		}
	})
}
