package artifacts

import (
	"strings"
	"testing"
	"time"
)

// Probe (lane B, 2026-10-29): what Expires does a per-put TTL produce?
func TestProbeArtifactTTL(t *testing.T) {
	dir := t.TempDir()
	s, err := Open(Options{Dir: dir, TTL: 24 * time.Hour})
	if err != nil { t.Fatal(err) }
	defer s.Close()
	cases := []struct{ name string; ttl time.Duration }{
		{"default-zero", 0},
		{"negative", -time.Hour},
		{"hundred-years", 876000 * time.Hour},
		{"max-parseable-ish", 2500000 * time.Hour},
	}
	for _, c := range cases {
		m, err := s.Put(strings.NewReader("x-"+c.name), PutOptions{Name: c.name + ".txt", TTL: c.ttl})
		if err != nil { t.Fatalf("%s: %v", c.name, err) }
		t.Logf("%s ttl=%v created=%s expires=%s delta_years=%.1f", c.name, c.ttl, m.Created.Format("2006-01-02"), m.Expires.Format("2006-01-02"), m.Expires.Sub(m.Created).Hours()/8760)
	}
	// The handler's parse step, verbatim semantics:
	for _, raw := range []string{"876000h", "2500000h", "99999999999999999999h", "-1h", "0s"} {
		d, err := time.ParseDuration(raw)
		t.Logf("ParseDuration(%q) = %v, err=%v", raw, d, err)
	}
}
