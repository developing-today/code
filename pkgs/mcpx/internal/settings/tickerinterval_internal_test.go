package settings

import (
	"strings"
	"testing"
)

// The ticker-interval guard is a class-of-bug guard: it iterates the
// authoritative tickerIntervalSettings table itself, so a future member
// of the table is covered the moment it is added, with no second list
// to keep in sync (#3467).
func TestEveryTickerIntervalSettingRefusesNonPositiveValues(t *testing.T) {
	sch, err := New(Registry())
	if err != nil {
		t.Fatal(err)
	}
	if len(tickerIntervalSettings) == 0 {
		t.Fatal("tickerIntervalSettings is empty; the guard would cover nothing")
	}
	// The three settings passed directly to time.NewTicker today must be
	// members; dropping one from the table fails here.
	for _, path := range []string{"daemon.reapInterval", "daemon.saveInterval", "http.ssePing"} {
		if !tickerIntervalSettings[path] {
			t.Errorf("%s is passed to time.NewTicker but missing from tickerIntervalSettings", path)
		}
	}
	for path := range tickerIntervalSettings {
		s, ok := sch.Lookup(path)
		if !ok {
			t.Errorf("%s is in tickerIntervalSettings but is not a registered setting", path)
			continue
		}
		if s.Kind != KindDuration {
			t.Errorf("%s is in tickerIntervalSettings but has kind %v, not KindDuration", path, s.Kind)
			continue
		}
		for _, raw := range []string{"0s", "-5s", "0", "-1ns"} {
			err := Validate(*s, raw)
			if err == nil {
				t.Errorf("%s=%q: want a refusal for a non-positive ticker interval", path, raw)
				continue
			}
			if !strings.Contains(err.Error(), path) {
				t.Errorf("%s=%q: refusal should name the setting, got %v", path, raw, err)
			}
		}
		for _, raw := range []string{"1ms", "30s", s.Default} {
			if err := Validate(*s, raw); err != nil {
				t.Errorf("%s=%q: want acceptance for a positive interval, got %v", path, raw, err)
			}
		}
	}
}
