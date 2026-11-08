package settings_test

import (
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

func protoTasksMaxTTLSetting(t *testing.T) settings.Setting {
	t.Helper()
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	s, ok := sch.Lookup("protoTasks.maxTTL")
	if !ok {
		t.Fatal("protoTasks.maxTTL is not a setting")
	}
	return *s
}

func TestProtoTasksMaxTTLRefusesPositiveSubMillisecondValues(t *testing.T) {
	// Store.Start clamps in whole milliseconds; a positive sub-millisecond
	// maximum would truncate to 0 and delete every new task at once, so
	// Validate refuses it at load (#2498). Removing the branch in
	// value.go must fail this test.
	s := protoTasksMaxTTLSetting(t)
	for _, raw := range []string{"1ns", "999ns", "1us", "500us", "999us", "0.5ms", "1500ns"} {
		err := settings.Validate(s, raw)
		if err == nil {
			t.Errorf("%q: want a refusal for a positive sub-millisecond maxTTL", raw)
			continue
		}
		if !strings.Contains(err.Error(), "at least 1ms") {
			t.Errorf("%q: refusal should name the 1ms floor, got %v", raw, err)
		}
	}
}

func TestProtoTasksMaxTTLsBoundaryAndFallbacks(t *testing.T) {
	s := protoTasksMaxTTLSetting(t)
	// The 1ms boundary itself is accepted, as is everything above it.
	for _, raw := range []string{"1ms", "2ms", "1500us", "90s", "1h"} {
		if err := settings.Validate(s, raw); err != nil {
			t.Errorf("%q: want acceptance at or above 1ms, got %v", raw, err)
		}
	}
	// Zero and negative values are the fallback spellings: the seams treat
	// a non-positive MaxTTL as tasks.DefaultMaxTTL, so validation leaves
	// them alone.
	for _, raw := range []string{"0", "0s", "-1ms", "-1s"} {
		if err := settings.Validate(s, raw); err != nil {
			t.Errorf("%q: want acceptance (non-positive falls back to the default), got %v", raw, err)
		}
	}
}

func TestSubMillisecondRefusalIsScopedToMaxTTL(t *testing.T) {
	// The refusal names one setting's semantics, not durations in general:
	// another duration setting keeps accepting sub-millisecond values.
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	poll, ok := sch.Lookup("protoTasks.pollInterval")
	if !ok {
		t.Fatal("protoTasks.pollInterval is not a setting")
	}
	if err := settings.Validate(*poll, "500us"); err != nil {
		t.Errorf("protoTasks.pollInterval %q: want acceptance, got %v", "500us", err)
	}
}

func TestProtoTasksCancelGraceSettingDefaultAndValidation(t *testing.T) {
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatal(err)
	}
	s, ok := sch.Lookup("protoTasks.cancelGrace")
	if !ok {
		t.Fatal("protoTasks.cancelGrace is not a setting")
	}
	if s.Default != "5s" {
		t.Errorf("default = %q, want 5s", s.Default)
	}
	for _, raw := range []string{"1ms", "5s", "30s"} {
		if err := settings.Validate(*s, raw); err != nil {
			t.Errorf("%q: want valid duration, got %v", raw, err)
		}
	}
}
