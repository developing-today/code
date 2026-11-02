package cli

// A snippet's own shebang must survive the prefix splice: dispatch reads
// the first line to pick the interpreter, and a prefix line in front of a
// shebang sent a valid shell snippet to the JavaScript harness (#2611
// review).

import (
	"strings"
	"testing"
)

func TestInlineSourceKeepsAShebangFirst(t *testing.T) {
	sh := scriptShape{prefix: []string{`echo before`}}
	got := sh.inlineSource([]string{"#!/bin/sh\necho hello"})
	if !strings.HasPrefix(got, "#!/bin/sh\n") {
		t.Fatalf("the snippet's shebang must stay on line 1:\n%s", got)
	}
	if !strings.Contains(got, "echo before") || !strings.Contains(got, "echo hello") {
		t.Fatalf("prefix and body must both survive:\n%s", got)
	}
}

func TestInlineSourceWithoutShebangIsUnchanged(t *testing.T) {
	sh := scriptShape{prefix: []string{"const a = 1"}, suffix: []string{"const b = 2"}}
	got := sh.inlineSource([]string{"console.log(1)"})
	want := "const a = 1\nconsole.log(1)\nconst b = 2"
	if got != want {
		t.Errorf("got %q, want %q", got, want)
	}
}
