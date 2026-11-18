package settings_test

import (
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

// daemon.inline widened from bool to off|fallback|always (#1868 point 2):
// the shipped boolean spellings keep their meaning through EnumAliases.
func TestDaemonInlineBooleanSpellingsKeepTheirMeaning(t *testing.T) {
	sch, err := settings.New(settings.Registry())
	if err != nil {
		t.Fatalf("registry schema: %v", err)
	}
	cases := map[string]string{
		"true":     "fallback",
		"false":    "off",
		"off":      "off",
		"fallback": "fallback",
		"always":   "always",
	}
	for raw, want := range cases {
		set := settings.NewSet(sch)
		if err := set.SetRuntime("daemon.inline", raw); err != nil {
			t.Fatalf("SetRuntime(daemon.inline, %q): %v", raw, err)
		}
		if got := set.String("daemon.inline"); got != want {
			t.Errorf("daemon.inline=%q resolved to %q, want %q", raw, got, want)
		}
	}
	set := settings.NewSet(sch)
	if got := set.String("daemon.inline"); got != "off" {
		t.Errorf("daemon.inline default = %q, want off", got)
	}
	if err := set.SetRuntime("daemon.inline", "sometimes"); err == nil {
		t.Error("SetRuntime(daemon.inline, sometimes) succeeded, want a validation error")
	}
}
