package settings_test

import (
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

func lookupSetting(t *testing.T, path string) settings.Setting {
	t.Helper()
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	s, ok := sch.Lookup(path)
	if !ok {
		t.Fatalf("%s is not a setting", path)
	}
	return *s
}

// The three settings below are passed directly to time.NewTicker, which
// panics on a non-positive period, so Validate must refuse zero and
// negative values for them (#3467). Removing the tickerIntervalSettings
// branch in value.go must fail this test.
func TestTickerIntervalsRefuseNonPositiveValues(t *testing.T) {
	for _, path := range []string{"daemon.reapInterval", "daemon.saveInterval", "http.ssePing"} {
		s := lookupSetting(t, path)
		for _, raw := range []string{"0s", "-5s", "0", "-1ns"} {
			err := settings.Validate(s, raw)
			if err == nil {
				t.Errorf("%s=%q: want a refusal for a non-positive ticker interval", path, raw)
				continue
			}
			if !strings.Contains(err.Error(), path) {
				t.Errorf("%s=%q: refusal should name the setting, got %v", path, raw, err)
			}
		}
		for _, raw := range []string{"1ms", "30s", s.Default} {
			if err := settings.Validate(s, raw); err != nil {
				t.Errorf("%s=%q: want acceptance for a positive interval, got %v", path, raw, err)
			}
		}
	}
}

// Positivity is per-setting, not blanket: daemon.idleExit=0s means
// "never exit" and must stay valid (#3467).
func TestIdleExitZeroStaysValid(t *testing.T) {
	s := lookupSetting(t, "daemon.idleExit")
	if err := settings.Validate(s, "0s"); err != nil {
		t.Errorf("daemon.idleExit=0s: want acceptance (zero means never), got %v", err)
	}
}

// Durations whose readers fall back to a default when non-positive
// (artifacts.gcInterval, transport.sseKeepAlive) are not ticker
// intervals; Validate must keep accepting zero for them so the guarded
// fallback behaviour stays intact (#3467).
func TestGuardedFallbackDurationsStillAcceptZero(t *testing.T) {
	for _, path := range []string{"artifacts.gcInterval", "transport.sseKeepAlive"} {
		s := settings.Setting{Path: path, Kind: settings.KindDuration, Default: "30s"}
		if err := settings.Validate(s, "0s"); err != nil {
			t.Errorf("%s=0s: want acceptance (reader defaults it), got %v", path, err)
		}
	}
}
