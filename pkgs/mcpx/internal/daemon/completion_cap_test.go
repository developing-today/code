package daemon

import (
	"encoding/json"
	"fmt"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

// #2878: the daemon /v1/complete surface never applied the 100-value
// spec cap. The shared computation lives in internal/settings; these
// tests pin the daemon's use of it on both reply paths.

func TestEffectiveCompletionCap(t *testing.T) {
	for raw, want := range map[int]int{
		250: 100, 101: 100, 100: 100, 99: 99, 50: 50, 1: 1,
		0: 100, -5: 100,
	} {
		if got := settings.EffectiveCompletionCap(raw); got != want {
			t.Errorf("EffectiveCompletionCap(%d) = %d, want %d", raw, got, want)
		}
	}
	if settings.SpecCompletionCap != 100 {
		t.Errorf("SpecCompletionCap = %d, want 100", settings.SpecCompletionCap)
	}
}

func manyPrompts(n int) []string {
	names := make([]string, n)
	for i := range names {
		names[i] = fmt.Sprintf("p%03d", i)
	}
	return names
}

// The local fallback, fed the cap exactly as handleComplete computes
// it: no setting value can push a reply past 100, 0/unset means the
// default 100 (not unlimited), and a small setting still lowers it.
func TestLocalCompletionSharedCap(t *testing.T) {
	req := completeReq{}
	req.Ref.Type = "ref/prompt"
	for raw, want := range map[int]int{250: 100, 0: 100, -1: 100, 100: 100, 50: 50} {
		got := localCompletion(promptsOf(manyPrompts(150)...), nil, req,
			settings.EffectiveCompletionCap(raw))
		vals := completeValues(t, got)
		if len(vals) != want {
			t.Errorf("setting %d: %d values, want %d", raw, len(vals), want)
		}
		if got["total"] != 150 {
			t.Errorf("setting %d: total = %v, want 150 (full ranked count)", raw, got["total"])
		}
		if got["hasMore"] != true {
			t.Errorf("setting %d: hasMore = %v, want true", raw, got["hasMore"])
		}
	}
}

// The upstream passthrough: an over-cap reply is truncated to the
// shared cap with a consistent total/hasMore; anything the cap cannot
// read passes through byte-identical.
func TestCapUpstreamCompletion(t *testing.T) {
	values := make([]string, 150)
	for i := range values {
		values[i] = fmt.Sprintf("v%03d", i)
	}
	payload, _ := json.Marshal(map[string]any{
		"values": values, "total": 150, "hasMore": false,
	})
	var out struct {
		Values  []string `json:"values"`
		Total   int      `json:"total"`
		HasMore bool     `json:"hasMore"`
	}
	if err := json.Unmarshal(capUpstreamCompletion(payload, 100), &out); err != nil {
		t.Fatal(err)
	}
	if len(out.Values) != 100 || out.Total != 150 || !out.HasMore {
		t.Errorf("capped = %d values, total %d, hasMore %v; want 100/150/true",
			len(out.Values), out.Total, out.HasMore)
	}

	small, _ := json.Marshal(map[string]any{
		"values": []string{"a", "b"}, "total": 2, "hasMore": false,
	})
	if got := capUpstreamCompletion(small, 100); string(got) != string(small) {
		t.Errorf("under-cap payload changed: %s", got)
	}
	for _, raw := range []string{`{not json`, `{"foo":1}`, `{"values":"nope"}`} {
		if got := capUpstreamCompletion(json.RawMessage(raw), 100); string(got) != raw {
			t.Errorf("undecodable payload %q changed to %q", raw, got)
		}
	}
}
