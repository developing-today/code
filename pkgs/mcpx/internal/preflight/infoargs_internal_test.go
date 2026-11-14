package preflight

import (
	"strings"
	"testing"
)

// TestInfoArgsStripsCachedOnly pins the #8457/#10289 contract:
// `deno info` rejects --cached-only outright, so it must not
// reach the info argv; its presence is reported instead, so the
// caller can restrain the subprocess environment. --no-remote is
// NOT substituted: on the info surface it refuses even cached
// remote modules, which --cached-only permits.
func TestInfoArgsStripsCachedOnly(t *testing.T) {
	argv, cacheOnly := infoArgs([]string{"--cached-only"})
	if !cacheOnly {
		t.Fatal("cacheOnly not reported for --cached-only")
	}
	for _, a := range argv {
		if a == "--cached-only" || a == "--no-remote" {
			t.Fatalf("info argv carries %q: %v", a, argv)
		}
	}

	argv, cacheOnly = infoArgs([]string{"--cached-only=true", "--frozen"})
	if !cacheOnly {
		t.Fatal("cacheOnly not reported for --cached-only=true")
	}
	if len(argv) != 1 || argv[0] != "--frozen" {
		t.Fatalf("unrelated resolution args must survive: %v", argv)
	}

	if _, cacheOnly = infoArgs([]string{"--frozen"}); cacheOnly {
		t.Fatal("cacheOnly reported without --cached-only")
	}
	if _, cacheOnly = infoArgs(nil); cacheOnly {
		t.Fatal("cacheOnly reported for no args")
	}
}

// TestCacheOnlyEnvForcesTheDeadProxy: every proxy spelling is
// forced to the dead endpoint and the bypass lists are emptied,
// whatever the inherited environment says — a NO_PROXY naming a
// registry would silently restore the fetching cache-only
// resolution exists to refuse.
func TestCacheOnlyEnvForcesTheDeadProxy(t *testing.T) {
	in := []string{
		"PATH=/usr/bin",
		"HTTPS_PROXY=http://proxy.corp:8080",
		"NO_PROXY=registry.npmjs.org,deno.land",
		"DENO_DIR=/cache",
	}
	out := cacheOnlyEnv(in)
	got := map[string]string{}
	for _, kv := range out {
		k, v, _ := strings.Cut(kv, "=")
		if prev, dup := got[k]; dup {
			t.Fatalf("duplicate env key %s (%q, %q)", k, prev, v)
		}
		got[k] = v
	}
	for _, key := range []string{"HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy"} {
		if got[key] != cacheOnlyProxy {
			t.Errorf("%s = %q, want the dead proxy", key, got[key])
		}
	}
	for _, key := range []string{"NO_PROXY", "no_proxy"} {
		if got[key] != "" {
			t.Errorf("%s = %q, want emptied", key, got[key])
		}
	}
	if got["PATH"] != "/usr/bin" || got["DENO_DIR"] != "/cache" {
		t.Errorf("unrelated variables must pass through: %v", out)
	}
}
