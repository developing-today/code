package execsvc

import (
	"context"
	"strings"
	"testing"
	"time"
)

// #2398: the per-request timeout override may lower the operator's
// bound, never remove or raise it.
func TestResolveExecTimeout(t *testing.T) {
	const limit = 120 * time.Second
	cases := []struct {
		name      string
		limit     time.Duration
		requested string
		want      time.Duration
		wantErr   string
	}{
		{"absent keeps operator limit", limit, "", limit, ""},
		{"below limit applies", limit, "30s", 30 * time.Second, ""},
		{"at limit applies", limit, "120s", limit, ""},
		{"above limit clamps", limit, "876000h", limit, ""},
		{"zero refused", limit, "0", 0, "must be positive"},
		{"zero duration refused", limit, "0s", 0, "must be positive"},
		{"negative refused", limit, "-1s", 0, "must be positive"},
		{"huge negative refused", limit, "-100h", 0, "must be positive"},
		{"unparseable refused", limit, "soon", 0, "invalid duration"},
		{"operator no-limit, positive applies unclamped", 0, "876000h", 876000 * time.Hour, ""},
		{"operator no-limit, absent stays unbounded", 0, "", 0, ""},
		{"operator no-limit, non-positive still refused", 0, "0s", 0, "must be positive"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			got, err := resolveExecTimeout(c.limit, c.requested)
			if c.wantErr != "" {
				if err == nil || !strings.Contains(err.Error(), c.wantErr) {
					t.Fatalf("resolveExecTimeout(%v, %q) err = %v, want containing %q", c.limit, c.requested, err, c.wantErr)
				}
				return
			}
			if err != nil {
				t.Fatalf("resolveExecTimeout(%v, %q): %v", c.limit, c.requested, err)
			}
			if got != c.want {
				t.Fatalf("resolveExecTimeout(%v, %q) = %v, want %v", c.limit, c.requested, got, c.want)
			}
		})
	}
}

// The refusal must fire at the Run entry point itself, before any
// runtime or client work: a bare Service with only Limits set reaches
// the override check first.
func TestRunRefusesNonPositiveTimeoutOverride(t *testing.T) {
	lim := DefaultLimits()
	lim.Timeout = 120 * time.Second
	svc := &Service{Limits: lim}
	for _, bad := range []string{"0", "0s", "-1s"} {
		_, err := svc.Run(context.Background(), Request{
			Source: "export {}",
			Opts:   Options{Timeout: bad},
		}, func(Frame) error { return nil })
		if err == nil || !strings.Contains(err.Error(), "must be positive") {
			t.Errorf("timeout %q: err = %v, want a must-be-positive refusal", bad, err)
		}
	}
}
