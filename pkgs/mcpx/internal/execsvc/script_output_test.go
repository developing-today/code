package execsvc

import (
	"encoding/json"
	"reflect"
	"strings"
	"testing"
)

// Oracle: only the four exact unsupported top-level effect names are reported.
// Values are inert fixture data, including null, and map/source order is irrelevant.
func TestScriptOutputDroppedEffects(t *testing.T) {
	cases := []struct {
		name, text string
		dropped    []string
	}{
		{"all", `{"run":"fixture","propose":{"value":1},"refuse":false,"ask":null,"advise":"note"}`, []string{"ask", "refuse", "propose", "run"}},
		{"nulls", `{"ask":null,"refuse":null,"propose":null,"run":null}`, []string{"ask", "refuse", "propose", "run"}},
		{"duplicate", `{"run":1,"ask":2,"run":3}`, []string{"ask", "run"}},
		{"nested", `{"apply":{"ask":1,"run":2},"advise":"note"}`, nil},
		{"exact", `{"Ask":1,"RUN":2,"proposal":3,"unknown":4}`, nil},
		{"one", `{"propose":false}`, []string{"propose"}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			var r hookRun
			r.readScriptOutput(tc.text)
			if !reflect.DeepEqual(r.Frame.Dropped, tc.dropped) {
				t.Fatalf("dropped=%v want %v", r.Frame.Dropped, tc.dropped)
			}
		})
	}
}

// Oracle: the previous typed decoder and fallback must produce byte-identical
// apply data and identical advice for every input, even when JSON decoding fails.
func TestScriptOutputPreservesDecoder(t *testing.T) {
	for _, text := range []string{"", "  note  ", "null", "[]", "42", `"note"`, `{"ask":1`, `{"ask":1,"advise":42}`, `{"advise":"ok","apply":null}`, `{"apply":{"value":7},"run":"fixture"}`} {
		t.Run(text, func(t *testing.T) {
			var want hookOutput
			trimmed := strings.TrimSpace(text)
			if trimmed != "" {
				if err := json.Unmarshal([]byte(trimmed), &want); err != nil {
					want = hookOutput{Advise: trimmed}
				}
			}
			var r hookRun
			r.readScriptOutput(text)
			if !reflect.DeepEqual(r.Out, want) || r.Frame.Advice != want.Advise {
				t.Fatalf("output=%+v advice=%q want %+v", r.Out, r.Frame.Advice, want)
			}
			var object map[string]json.RawMessage
			var typed hookOutput
			if json.Unmarshal([]byte(trimmed), &typed) != nil || json.Unmarshal([]byte(trimmed), &object) != nil || object == nil {
				if len(r.Frame.Dropped) != 0 {
					t.Fatalf("fallback reported keys: %v", r.Frame.Dropped)
				}
			}
		})
	}
	r := hookRun{Failed: true}
	r.readScriptOutput(`{"ask":1,"advise":"note","apply":2}`)
	if !reflect.DeepEqual(r.Out, hookOutput{}) || len(r.Frame.Dropped) != 0 || r.Frame.Advice != "" {
		t.Fatalf("failed hook parsed output: %+v", r)
	}
}

func TestScriptOutputDroppedPreservesPhaseApplication(t *testing.T) {
	for _, phase := range []string{HookBefore, HookSuffix, HookOnSuccess, HookOnError} {
		t.Run(phase, func(t *testing.T) {
			var r hookRun
			r.readScriptOutput(`{"run":"fixture","ask":null,"apply":7,"advise":"note"}`)
			res := Result{Result: float64(2), ResultPresent: true}
			if phase == HookOnError {
				res.Error = "fixture error"
				res.ExitCode = 1
			}
			original := applyHookResult(&r, phase, &res)
			wantDropped := []string{"ask", "run"}
			if effectFor(phase) == "apply" {
				if original == nil || res.Result != float64(7) || res.Error != "" || res.ExitCode != 0 {
					t.Fatalf("apply changed semantics: %+v original=%s", res, original)
				}
			} else {
				wantDropped = append(wantDropped, "apply")
				if original != nil || res.Result != float64(2) {
					t.Fatalf("advice changed result: %+v", res)
				}
			}
			if !reflect.DeepEqual(r.Frame.Dropped, wantDropped) || r.Frame.Advice != "note" {
				t.Fatalf("frame=%+v", r.Frame)
			}
		})
	}
}
