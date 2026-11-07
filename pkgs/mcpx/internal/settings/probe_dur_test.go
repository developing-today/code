package settings_test

import (
	"testing"
	"time"

	"github.com/dezren39/mcpx/internal/settings"
)

func TestProbeNonPositiveDurationAcceptedThenTickerPanics(t *testing.T) {
	s := settings.NewSet(schema(t))
	for _, tc := range []struct{ path, raw string }{
		{"daemon.reapInterval", "-5s"},
		{"daemon.reapInterval", "0s"},
		{"daemon.saveInterval", "-1s"},
		{"http.ssePing", "-1s"},
		{"http.ssePing", "0s"},
	} {
		if err := s.SetRuntime(tc.path, tc.raw); err != nil {
			t.Logf("SetRuntime(%s, %q) refused: %v", tc.path, tc.raw, err)
			continue
		}
		d := s.Duration(tc.path)
		t.Logf("SetRuntime(%s, %q) ACCEPTED; Duration()=%v", tc.path, tc.raw, d)
		func() {
			defer func() {
				if r := recover(); r != nil {
					t.Logf("time.NewTicker(%v) PANICS: %v", d, r)
				}
			}()
			tk := time.NewTicker(d)
			tk.Stop()
			t.Logf("time.NewTicker(%v) ok", d)
		}()
		s.ClearRuntime(tc.path)
	}
	// Direct Validate on the real registry declarations.
	sch := schema(t)
	for _, path := range []string{"daemon.reapInterval", "daemon.saveInterval", "http.ssePing"} {
		decl, ok := sch.Lookup(path)
		if !ok {
			t.Fatalf("no decl for %s", path)
		}
		for _, raw := range []string{"-5s", "0s"} {
			t.Logf("Validate(%s decl, %q) err=%v", path, raw, settings.Validate(*decl, raw))
		}
	}
}
