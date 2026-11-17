package cli

import "testing"

// TestScriptCheckForPerCommandDefault is #1868 point 4: run defaults
// script.check on (types) when nothing was given, exec stays at the
// resolved (registry-default off) value, and an explicitly given
// value always wins for both commands. Oracle: the acceptance text
// of issue #1868 point 4, case by case.
func TestScriptCheckForPerCommandDefault(t *testing.T) {
	for _, tc := range []struct {
		command, resolved string
		explicit          bool
		want              string
	}{
		{command: "run", resolved: "off", explicit: false, want: "types"},
		{command: "exec", resolved: "off", explicit: false, want: "off"},
		{command: "run", resolved: "off", explicit: true, want: "off"},
		{command: "run", resolved: "strict", explicit: true, want: "strict"},
		{command: "exec", resolved: "types", explicit: true, want: "types"},
		{command: "", resolved: "off", explicit: false, want: "off"},
	} {
		if got := scriptCheckFor(tc.command, tc.resolved, tc.explicit); got != tc.want {
			t.Errorf("scriptCheckFor(%q, %q, %v) = %q, want %q",
				tc.command, tc.resolved, tc.explicit, got, tc.want)
		}
	}
}
