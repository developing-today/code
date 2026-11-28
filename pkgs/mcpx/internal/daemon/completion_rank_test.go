package daemon

import (
	"reflect"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

func promptsOf(names ...string) []mcpclient.Prompt {
	out := make([]mcpclient.Prompt, len(names))
	for i, n := range names {
		out[i] = mcpclient.Prompt{Name: n}
	}
	return out
}

func resourcesOf(uris ...string) []mcpclient.Resource {
	out := make([]mcpclient.Resource, len(uris))
	for i, u := range uris {
		out[i] = mcpclient.Resource{URI: u}
	}
	return out
}

func completeValues(t *testing.T, got map[string]any) []string {
	t.Helper()
	vals, ok := got["values"].([]string)
	if !ok {
		t.Fatalf("values = %T %v", got["values"], got["values"])
	}
	return vals
}

// The three tiers in order, for prompts: exact, then prefix (shorter
// first), then substring (by first-match position, then alphabetical).
// Candidates are listed interleaved alphabetically so alphabetical
// order cannot masquerade as tier order.
func TestLocalCompletionTiersPrompts(t *testing.T) {
	req := completeReq{}
	req.Ref.Type = "ref/prompt"
	req.Argument.Value = "alp"
	got := localCompletion(promptsOf(
		"zzalpzz", // substring, position 2
		"alpine",  // prefix, 6 runes
		"xalp",    // substring, position 1
		"alp",     // exact
		"beta",    // no match
		"alps",    // prefix, 4 runes
		"balp",    // substring, position 1, alphabetically before xalp
	), nil, req, 0)
	want := []string{"alp", "alps", "alpine", "balp", "xalp", "zzalpzz"}
	if vals := completeValues(t, got); !reflect.DeepEqual(vals, want) {
		t.Fatalf("values = %v, want %v", vals, want)
	}
	if got["total"] != 6 || got["hasMore"] != false {
		t.Fatalf("total/hasMore = %v/%v, want 6/false", got["total"], got["hasMore"])
	}
}

// The same tiers for resource template URIs, including a mid-name
// substring hit that the old prefix-only fallback returned nothing for.
func TestLocalCompletionTiersResources(t *testing.T) {
	req := completeReq{}
	req.Ref.Type = "ref/resource"
	req.Argument.Value = "temp"
	got := localCompletion(nil, resourcesOf(
		"file:///var/temp", // substring, position 10
		"template",         // prefix, 8 runes
		"temp",             // exact
		"other",            // no match
		"temp2",            // prefix, 5 runes
		"atemp",            // substring, position 1
	), req, 0)
	want := []string{"temp", "temp2", "template", "atemp", "file:///var/temp"}
	if vals := completeValues(t, got); !reflect.DeepEqual(vals, want) {
		t.Fatalf("values = %v, want %v", vals, want)
	}
}

// A mid-name substring hit: "pin" appears inside "alpine" but is not a
// prefix, so the pre-#2388 fallback found nothing.
func TestLocalCompletionSubstringHit(t *testing.T) {
	req := completeReq{}
	req.Ref.Type = "ref/prompt"
	req.Argument.Value = "pin"
	got := localCompletion(promptsOf("alpine", "beta"), nil, req, 0)
	if vals := completeValues(t, got); !reflect.DeepEqual(vals, []string{"alpine"}) {
		t.Fatalf("values = %v, want [alpine]", vals)
	}
}

// Matching is case-folded: an upper-case value still hits the exact
// tier, and exact beats prefix regardless of case.
func TestLocalCompletionCaseFolded(t *testing.T) {
	req := completeReq{}
	req.Ref.Type = "ref/prompt"
	req.Argument.Value = "ALP"
	got := localCompletion(promptsOf("alpine", "Alp"), nil, req, 0)
	if vals := completeValues(t, got); !reflect.DeepEqual(vals, []string{"Alp", "alpine"}) {
		t.Fatalf("values = %v, want [Alp alpine]", vals)
	}
}

// The cap applies AFTER ranking: the alphabetically-first candidate
// (a substring match) must not survive truncation ahead of the exact
// match, and total/hasMore report the full ranked match count.
func TestLocalCompletionCapAfterRanking(t *testing.T) {
	req := completeReq{}
	req.Ref.Type = "ref/prompt"
	req.Argument.Value = "alp"
	got := localCompletion(promptsOf("aalp", "alpine", "alp"), nil, req, 1)
	if vals := completeValues(t, got); !reflect.DeepEqual(vals, []string{"alp"}) {
		t.Fatalf("values = %v, want [alp]", vals)
	}
	if got["total"] != 3 || got["hasMore"] != true {
		t.Fatalf("total/hasMore = %v/%v, want 3/true", got["total"], got["hasMore"])
	}
}

// An empty value puts every candidate in the prefix tier: shorter
// names first, then alphabetical (triage settlement, task 3557).
func TestLocalCompletionEmptyValue(t *testing.T) {
	req := completeReq{}
	req.Ref.Type = "ref/prompt"
	req.Argument.Value = ""
	got := localCompletion(promptsOf("bbbb", "a", "cc", "dd"), nil, req, 0)
	want := []string{"a", "cc", "dd", "bbbb"}
	if vals := completeValues(t, got); !reflect.DeepEqual(vals, want) {
		t.Fatalf("values = %v, want %v", vals, want)
	}
	if got["total"] != 4 || got["hasMore"] != false {
		t.Fatalf("total/hasMore = %v/%v, want 4/false", got["total"], got["hasMore"])
	}
}

// Duplicate names in the cache are preserved, as before.
func TestLocalCompletionDuplicatesPreserved(t *testing.T) {
	req := completeReq{}
	req.Ref.Type = "ref/prompt"
	req.Argument.Value = "alp"
	got := localCompletion(promptsOf("alp", "alp", "alps"), nil, req, 0)
	if vals := completeValues(t, got); !reflect.DeepEqual(vals, []string{"alp", "alp", "alps"}) {
		t.Fatalf("values = %v, want [alp alp alps]", vals)
	}
}

// No matches: empty values, zero total, hasMore false.
func TestLocalCompletionNoMatch(t *testing.T) {
	req := completeReq{}
	req.Ref.Type = "ref/prompt"
	req.Argument.Value = "zzz"
	got := localCompletion(promptsOf("alpine"), nil, req, 0)
	if vals := completeValues(t, got); len(vals) != 0 {
		t.Fatalf("values = %v, want empty", vals)
	}
	if got["total"] != 0 || got["hasMore"] != false {
		t.Fatalf("total/hasMore = %v/%v, want 0/false", got["total"], got["hasMore"])
	}
}
