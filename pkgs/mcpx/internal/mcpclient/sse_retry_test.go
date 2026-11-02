package mcpclient

import (
	"strings"
	"testing"
	"time"
)

// Regression tests for #2368: the SSE retry field is server-controlled
// input. It must be clamped at the pinned 60s ceiling before the
// millisecond-to-Duration conversion, so no value can stall
// resume()/listen() for years or wrap to microseconds/negative.
func TestParseSSERetry(t *testing.T) {
	cases := []struct {
		name  string
		value string
		want  time.Duration
		ok    bool
	}{
		{"normal value unchanged", "3000", 3 * time.Second, true},
		{"zero", "0", 0, true},
		{"at ceiling", "60000", 60 * time.Second, true},
		{"just over ceiling", "60001", 60 * time.Second, true},
		{"ten years clamps", "315360000000", 60 * time.Second, true},
		{"would wrap to ~219 years clamps", "8640000000000000", 60 * time.Second, true},
		{"would wrap negative clamps", "10000000000000", 60 * time.Second, true},
		{"would wrap to microseconds clamps", "18446744073710", 60 * time.Second, true},
		{"int64 max clamps", "9223372036854775807", 60 * time.Second, true},
		{"beyond int64 ignored", "99999999999999999999999999", 0, false},
		{"non-numeric ignored", "abc", 0, false},
		{"negative ignored", "-5", 0, false},
		{"empty ignored", "", 0, false},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			got, ok := parseSSERetry(c.value)
			if ok != c.ok || got != c.want {
				t.Errorf("parseSSERetry(%q) = (%v, %v), want (%v, %v)", c.value, got, ok, c.want, c.ok)
			}
		})
	}
}

// The stream-level state carries the clamped value: a retry line in an
// SSE stream lands in sseState.retry at the ceiling, and an ignored
// value leaves it at zero (the wait sites then use the operator
// default, exactly as before).
func TestReadSSERetryClamp(t *testing.T) {
	tr := &HTTPTransport{}
	st := tr.readSSE(nil, strings.NewReader("retry: 315360000000\n\n"), nil)
	if st.retry != 60*time.Second {
		t.Errorf("readSSE retry = %v, want 60s (clamped)", st.retry)
	}
	st = tr.readSSE(nil, strings.NewReader("retry: 3000\n\n"), nil)
	if st.retry != 3*time.Second {
		t.Errorf("readSSE retry = %v, want 3s", st.retry)
	}
	st = tr.readSSE(nil, strings.NewReader("retry: -5\n\n"), nil)
	if st.retry != 0 {
		t.Errorf("readSSE retry = %v, want 0 (ignored, default applies)", st.retry)
	}
}
