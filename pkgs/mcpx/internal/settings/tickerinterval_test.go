package settings_test

import (
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

// The refusal behaviour for the ticker settings themselves is covered
// by TestEveryTickerIntervalSettingRefusesNonPositiveValues in
// tickerinterval_internal_test.go, which iterates the authoritative
// tickerIntervalSettings table (present and future members). The tests
// here pin the other half of the contract: the durations that are NOT
// ticker intervals keep their semantics.

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
