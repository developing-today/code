package conformance

import (
	"os"
	"strings"
	"testing"
)

func TestTaskVisibilityDocumentation(t *testing.T) {
	protocol := strings.Join(strings.Fields(readTaskDoc(t, "../../docs/protocol.md")), " ")
	for _, required := range []string{
		"Task IDs and visibility",
		"128 random bits",
		"anyone who learns an unbound task's ID can retrieve it",
		"An unbound task is never included in legacy `tasks/list`",
		"a connection without an identity gets an empty list",
		"must provide an identity",
		"The daemon's `/v1/tasks` endpoints use the same daemon-wide store",
	} {
		if !strings.Contains(protocol, required) {
			t.Errorf("docs/protocol.md is missing task-visibility disclosure %q", required)
		}
	}
	for _, path := range []string{"../../docs/cli.md", "../../docs/exec.md"} {
		if !strings.Contains(readTaskDoc(t, path), "128-bit") {
			t.Errorf("%s does not identify task IDs as 128-bit handles", path)
		}
	}
}

func readTaskDoc(t *testing.T, path string) string {
	t.Helper()
	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return string(b)
}
