package placement

import (
	"encoding/json"
	"fmt"
	"math"
	"os"
	"time"
)

// ReadQuotaCurrent is the quota-current producer for the live wiring of
// issue #2732: it reads the quota gatherer's quota-current.json
// (pm-stack/aggregator/scripts/gather-quota.ts writes it every 15 minutes)
// and returns one pool's current monthly utilization percent together with
// the read's timestamp, for the monthly pace guard. The caller passes the
// policy's B5 freshness bound (60 minutes) as maxAge.
//
// The reader mirrors the gatherer reader's semantics in
// pm-stack/aggregator/src/quota-current.ts, so the two can never drift
// apart on what counts as a usable read:
//
//   - Only an entry whose state is "ok" can supply a number. An error,
//     lapsed, or stale entry never becomes a percent; a missing or stale
//     read is an error, not a zero.
//   - An entry the gatherer retained after a failed tick
//     (retainedFromFailedTick set) is stale by definition — the
//     provider's latest run failed, however recent its measuredAt is
//     (#1268 in the aggregator's entryState). It never supplies a
//     number and never shadows an older genuine ok read.
//   - Several entries can share a serviceId (one per credential): the
//     newest ok entry wins.
//   - A read stamped more than quotaCurrentFutureSkew in the future is
//     not fresh at all (clock skew, #1541 in the aggregator).
//   - The monthly percent is the entry's "monthly" window pctOfLimit.
//     It must be finite and inside [0, 100]: an out-of-range read drops,
//     never clamps, so a corrupt read can neither wall a healthy pool
//     nor report negative usage (#2582 in the aggregator).
//
// The evaluator still enforces the B5 gate on the returned timestamp, so
// a read that ages past the bound between this call and evaluation is
// refused there, not seated on stale data.
func ReadQuotaCurrent(path, pool string, now time.Time, maxAge time.Duration) (float64, time.Time, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return 0, time.Time{}, fmt.Errorf("quota-current: read %s: %w", path, err)
	}
	var doc quotaCurrentDoc
	if err := json.Unmarshal(raw, &doc); err != nil {
		return 0, time.Time{}, fmt.Errorf("quota-current: parse %s: %w", path, err)
	}
	if doc.Version != 1 {
		return 0, time.Time{}, fmt.Errorf("quota-current: %s: unsupported version %d, want 1", path, doc.Version)
	}
	var (
		entriesForPool int
		okEntries      int
		newest         *quotaCurrentEntry
		newestAt       time.Time
	)
	for i := range doc.Providers {
		e := &doc.Providers[i]
		if e.ServiceID != pool {
			continue
		}
		entriesForPool++
		if e.State != "ok" {
			continue
		}
		okEntries++
		if e.RetainedFromFailedTick != "" {
			// Retained after a failed gatherer tick: stale by
			// definition (#1268), never a usable read.
			continue
		}
		measuredAt, err := time.Parse(time.RFC3339, e.MeasuredAt)
		if err != nil {
			continue
		}
		if measuredAt.After(now.Add(quotaCurrentFutureSkew)) {
			continue
		}
		if now.Sub(measuredAt) > maxAge {
			continue
		}
		if newest == nil || measuredAt.After(newestAt) {
			newest, newestAt = e, measuredAt
		}
	}
	switch {
	case entriesForPool == 0:
		return 0, time.Time{}, fmt.Errorf("quota-current: no read for pool %q in %s", pool, path)
	case okEntries == 0:
		return 0, time.Time{}, fmt.Errorf("quota-current: no ok read for pool %q (all %d entries failed or lapsed)", pool, entriesForPool)
	case newest == nil:
		return 0, time.Time{}, fmt.Errorf("quota-current: no fresh ok read for pool %q within %s (stale)", pool, maxAge)
	}
	for _, w := range newest.Windows {
		if w.Name != "monthly" {
			continue
		}
		if w.PctOfLimit == nil || math.IsNaN(*w.PctOfLimit) || math.IsInf(*w.PctOfLimit, 0) || *w.PctOfLimit < 0 || *w.PctOfLimit > 100 {
			return 0, time.Time{}, fmt.Errorf("quota-current: pool %q monthly pct %v out of [0, 100]; dropping the read, never clamping", pool, pctOrNil(w.PctOfLimit))
		}
		return *w.PctOfLimit, newestAt, nil
	}
	return 0, time.Time{}, fmt.Errorf("quota-current: freshest ok read for pool %q carries no monthly window", pool)
}

// quotaCurrentFutureSkew is the clock-skew tolerance for a gatherer
// entry's measuredAt, mirroring QUOTA_CURRENT_FUTURE_SKEW_MS in
// pm-stack/aggregator/src/quota-current.ts (#1541).
const quotaCurrentFutureSkew = 60 * time.Second

func pctOrNil(p *float64) any {
	if p == nil {
		return nil
	}
	return *p
}

// quotaCurrentDoc is the subset of the gatherer's quota-current.json the
// placement reader needs; every other field is ignored at parse time.
type quotaCurrentDoc struct {
	Version   int                 `json:"version"`
	Providers []quotaCurrentEntry `json:"providers"`
}

type quotaCurrentEntry struct {
	ServiceID  string               `json:"serviceId"`
	State      string               `json:"state"`
	MeasuredAt string               `json:"measuredAt"`
	Windows    []quotaCurrentWindow `json:"windows"`
	// RetainedFromFailedTick is the gatherer's stamp (an ISO
	// timestamp) on an entry it carried over after a failed tick
	// (markRetained in pm-stack/aggregator/src/quota-gather-map.ts);
	// non-empty means the entry is retained, not freshly read.
	RetainedFromFailedTick string `json:"retainedFromFailedTick"`
}

type quotaCurrentWindow struct {
	Name       string   `json:"name"`
	PctOfLimit *float64 `json:"pctOfLimit"`
}
