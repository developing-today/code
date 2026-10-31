package execsvc_test

import (
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/execsvc"
)

// CODE-16 (#1937), Codex P2 on PR 2979: when large JSON stdout is
// capped, the parsed Result must not survive as a side channel that
// lets a renderer re-present the uncapped answer in full.
func TestTruncatedJSONStdoutClearsStdoutDerivedResult(t *testing.T) {
	svc := service(t, store(t))
	svc.Limits.StdoutLimit = 200
	res, _ := run(t, svc, `
import { installGlobals } from "./mcpx-client.ts";
installGlobals();
console.log(JSON.stringify({ data: "x".repeat(5000) }));
`, execsvc.Options{})

	if !res.StdoutTruncated {
		t.Fatal("expected StdoutTruncated")
	}
	if len(res.Stdout) > 200 {
		t.Fatalf("capped stdout is %d bytes, over the 200-byte cap", len(res.Stdout))
	}
	if res.Result != nil {
		t.Fatalf("stdout-derived Result survived truncation: %#v", res.Result)
	}
}

// Under the cap the stdout-derived Result is offered parsed, as
// before: the clearing above must only fire on truncation.
func TestUntruncatedJSONStdoutKeepsResult(t *testing.T) {
	svc := service(t, store(t))
	svc.Limits.StdoutLimit = 200
	res, _ := run(t, svc, `
import { installGlobals } from "./mcpx-client.ts";
installGlobals();
console.log(JSON.stringify({ total: 2 }));
`, execsvc.Options{})

	if res.StdoutTruncated {
		t.Fatal("under-cap output marked truncated")
	}
	m, ok := res.Result.(map[string]any)
	if !ok || m["total"] != float64(2) {
		t.Fatalf("JSON on stdout should be offered parsed, got %#v", res.Result)
	}
	if !strings.Contains(res.Stdout, "total") {
		t.Fatalf("stdout = %q", res.Stdout)
	}
}
