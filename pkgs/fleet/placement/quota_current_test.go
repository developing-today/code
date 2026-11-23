package placement

import (
	"os"
	"strconv"
	"strings"
	"testing"
	"time"
)

// TestReadQuotaCurrent is the wiring slice of issue #2732: placement-record
// must take its monthly-guard inputs from the quota gatherer's
// quota-current.json instead of hand-supplied answers, so the first wired
// seating call refuses a ≥97% pool from a live read. The reader is the
// producer the 14:14Z xaltha note on #2732 named as missing: it returns the
// freshest ok entry's "monthly" window pctOfLimit and its measuredAt, or an
// error — a stale, failed, or missing read never becomes a number, and an
// out-of-range percent drops instead of clamping (pm-stack's usablePct
// rule, #2582). Freshness is the policy's B5 gate (60 minutes), passed in
// by the caller.
func TestReadQuotaCurrent(t *testing.T) {
	now := time.Date(2026, 11, 23, 16, 0, 0, 0, time.UTC)
	maxAge := 60 * time.Minute

	file := func(entries string) string {
		return `{"version":1,"generatedAt":"2026-11-23T16:00:00Z","source":"gatherer","providers":[` + entries + `]}`
	}
	entry := func(serviceID, state, measuredAt, windows string) string {
		return `{"providerId":"` + serviceID + `","providerName":"` + serviceID + `","serviceId":"` + serviceID + `","state":"` + state + `","measuredAt":"` + measuredAt + `","source":"api","windows":[` + windows + `]}`
	}
	monthly := func(pct float64) string {
		return `{"name":"monthly","value":null,"pctOfLimit":` + strconv.FormatFloat(pct, 'f', -1, 64) + `,"valueLabel":null,"windowSeconds":null,"resetsAt":null,"measuredAt":"2026-11-23T15:50:00Z","source":"api","state":"ok"}`
	}
	writeFile := func(t *testing.T, content string) string {
		t.Helper()
		path := t.TempDir() + "/quota-current.json"
		if err := os.WriteFile(path, []byte(content), 0o600); err != nil {
			t.Fatalf("write fixture: %v", err)
		}
		return path
	}

	t.Run("fresh ok monthly read returns pct and read time", func(t *testing.T) {
		path := writeFile(t, file(entry("chatgpt-plan", "ok", "2026-11-23T15:50:00Z", monthly(98.4))))
		pct, readAt, err := ReadQuotaCurrent(path, "chatgpt-plan", now, maxAge)
		if err != nil {
			t.Fatalf("ReadQuotaCurrent: %v", err)
		}
		if pct != 98.4 {
			t.Fatalf("pct = %v, want 98.4", pct)
		}
		if got := readAt.UTC().Format(time.RFC3339); got != "2026-11-23T15:50:00Z" {
			t.Fatalf("readAt = %s, want 2026-11-23T15:50:00Z", got)
		}
	})

	t.Run("stale ok read is an error, never a number", func(t *testing.T) {
		path := writeFile(t, file(entry("chatgpt-plan", "ok", "2026-11-23T13:00:00Z", monthly(98.4))))
		if _, _, err := ReadQuotaCurrent(path, "chatgpt-plan", now, maxAge); err == nil {
			t.Fatal("expected a stale-read error for a 3h-old read under the 60m B5 gate")
		} else if !strings.Contains(err.Error(), "stale") {
			t.Fatalf("stale error %q does not say stale", err)
		}
	})

	t.Run("error-state entry yields no number", func(t *testing.T) {
		path := writeFile(t, file(entry("chatgpt-plan", "error", "2026-11-23T15:50:00Z", monthly(98.4))))
		if _, _, err := ReadQuotaCurrent(path, "chatgpt-plan", now, maxAge); err == nil {
			t.Fatal("expected an error when the only read for the pool failed")
		}
	})

	t.Run("no entry for the pool is an error", func(t *testing.T) {
		path := writeFile(t, file(entry("other-pool", "ok", "2026-11-23T15:50:00Z", monthly(12.0))))
		if _, _, err := ReadQuotaCurrent(path, "chatgpt-plan", now, maxAge); err == nil {
			t.Fatal("expected an error when the file carries no read for the pool")
		}
	})

	t.Run("fresh ok read without a monthly window is an error", func(t *testing.T) {
		fiveHour := `{"name":"5h","value":null,"pctOfLimit":40.0,"valueLabel":null,"windowSeconds":null,"resetsAt":null,"measuredAt":"2026-11-23T15:50:00Z","source":"api","state":"ok"}`
		path := writeFile(t, file(entry("chatgpt-plan", "ok", "2026-11-23T15:50:00Z", fiveHour)))
		if _, _, err := ReadQuotaCurrent(path, "chatgpt-plan", now, maxAge); err == nil {
			t.Fatal("expected an error when the freshest ok read carries no monthly window")
		}
	})

	t.Run("newest ok entry wins over an older one", func(t *testing.T) {
		old := entry("chatgpt-plan", "ok", "2026-11-23T15:00:00Z", monthly(50.0))
		recent := entry("chatgpt-plan", "ok", "2026-11-23T15:50:00Z", monthly(98.4))
		path := writeFile(t, file(old+","+recent))
		pct, _, err := ReadQuotaCurrent(path, "chatgpt-plan", now, maxAge)
		if err != nil {
			t.Fatalf("ReadQuotaCurrent: %v", err)
		}
		if pct != 98.4 {
			t.Fatalf("pct = %v, want the newest entry's 98.4", pct)
		}
	})

	t.Run("out-of-range monthly percent drops, never clamps", func(t *testing.T) {
		path := writeFile(t, file(entry("chatgpt-plan", "ok", "2026-11-23T15:50:00Z", monthly(150.0))))
		if _, _, err := ReadQuotaCurrent(path, "chatgpt-plan", now, maxAge); err == nil {
			t.Fatal("expected an error for a 150% monthly read (out of [0,100])")
		}
	})

	t.Run("malformed file is an error", func(t *testing.T) {
		path := writeFile(t, `{"version":2,"providers":[]}`)
		if _, _, err := ReadQuotaCurrent(path, "chatgpt-plan", now, maxAge); err == nil {
			t.Fatal("expected an error for a wrong-version quota-current document")
		}
	})

	t.Run("future read beyond skew tolerance is an error", func(t *testing.T) {
		path := writeFile(t, file(entry("chatgpt-plan", "ok", "2026-11-23T16:05:00Z", monthly(98.4))))
		if _, _, err := ReadQuotaCurrent(path, "chatgpt-plan", now, maxAge); err == nil {
			t.Fatal("expected an error for a read stamped 5m in the future")
		}
	})

	t.Run("missing file is an error", func(t *testing.T) {
		if _, _, err := ReadQuotaCurrent(t.TempDir()+"/absent.json", "chatgpt-plan", now, maxAge); err == nil {
			t.Fatal("expected an error for a missing quota-current file")
		}
	})
}

// TestQuotaCurrentWiresTheGuard composes the reader with the evaluator: a
// live 98.4% monthly read refuses the seating through the exact
// monthly-guard refusal line the weekly audit counts (CountRefusals), so
// the wired call's refusal is countable from dispatcher-log data.
func TestQuotaCurrentWiresTheGuard(t *testing.T) {
	p := testPolicy(t)
	now := time.Date(2026, 11, 23, 16, 0, 0, 0, time.UTC)
	doc := `{"version":1,"generatedAt":"2026-11-23T16:00:00Z","source":"gatherer","providers":[` +
		`{"providerId":"chatgpt-plan","providerName":"chatgpt-plan","serviceId":"chatgpt-plan","state":"ok","measuredAt":"2026-11-23T15:50:00Z","source":"api","windows":[` +
		`{"name":"monthly","value":null,"pctOfLimit":98.4,"valueLabel":null,"windowSeconds":null,"resetsAt":null,"measuredAt":"2026-11-23T15:50:00Z","source":"api","state":"ok"}]}]}`
	path := t.TempDir() + "/quota-current.json"
	if err := os.WriteFile(path, []byte(doc), 0o600); err != nil {
		t.Fatalf("write fixture: %v", err)
	}
	pct, readAt, err := ReadQuotaCurrent(path, "chatgpt-plan", now, 60*time.Minute)
	if err != nil {
		t.Fatalf("ReadQuotaCurrent: %v", err)
	}
	_, err = Evaluate(p, Answers{
		Task:           "queue task test / issue #2732",
		Q4Judgement:    true,
		PoolReadAt:     &readAt,
		PoolID:         "chatgpt-plan",
		PoolMonthlyPct: &pct,
	}, now)
	if err == nil {
		t.Fatal("expected the live 98.4% read to refuse the seating")
	}
	for _, want := range []string{"monthly-guard refusal", "chatgpt-plan", "98.4", "[source: monthly-guard]"} {
		if !strings.Contains(err.Error(), want) {
			t.Fatalf("refusal %q does not cite %q", err, want)
		}
	}
}
