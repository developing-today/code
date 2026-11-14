package cli

import (
	"strings"
	"testing"
)

// The advertised form puts flags after the name: mcpx api add petstore
// --spec ./petstore.json. Hoisting must keep each flag with its value,
// or the parser reads the spec path as the name.
func TestAPIHoistKeepsSpecPairedWithItsValue(t *testing.T) {
	in := []string{"petstore", "--spec", "./petstore.json", "--include", "pets,store"}
	got := hoistFlags(in, apiAddValuedFlags)
	want := []string{"--spec", "./petstore.json", "--include", "pets,store", "petstore"}
	if strings.Join(got, " ") != strings.Join(want, " ") {
		t.Fatalf("hoisted %v, want %v", got, want)
	}
}

func TestParseAPIHeaderSplitsKeyAndValue(t *testing.T) {
	k, v, err := parseAPIHeader("Authorization=Bearer abc==")
	if err != nil || k != "Authorization" || v != "Bearer abc==" {
		t.Fatalf("parseAPIHeader = %q, %q, %v", k, v, err)
	}
	if _, _, err := parseAPIHeader("no-equals-here"); err == nil {
		t.Fatal("a header without = was accepted")
	}
	if _, _, err := parseAPIHeader("=empty-key"); err == nil {
		t.Fatal("a header with an empty key was accepted")
	}
}
