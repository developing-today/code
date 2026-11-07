package config_test

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/codegen"
	"github.com/dezren39/mcpx/internal/config"
)

func TestProbeExplicitNamespaceValidity(t *testing.T) {
	for _, ns := range []string{"my-ns", "9lives", "a b", "foo.bar"} {
		c := &config.Config{MCPServers: map[string]*config.Server{
			"a": {Name: "a", Command: "x", Mcpx: &config.Extras{Namespace: ns}},
		}}
		r, err := c.Resolve("a")
		if err != nil {
			t.Logf("explicit namespace %q: REFUSED: %v", ns, err)
			continue
		}
		mod := codegen.Module([]codegen.Namespace{{Name: r.Namespace, Server: "a", Tools: []codegen.Tool{{Name: "ping"}}}}, "", "")
		for _, l := range strings.Split(mod, "\n") {
			if strings.Contains(l, r.Namespace) && strings.Contains(l, "= {") {
				t.Logf("explicit namespace %q: ACCEPTED; module line: %s", ns, strings.TrimSpace(l))
			}
		}
		p := filepath.Join(t.TempDir(), "client.ts")
		os.WriteFile(p, []byte(mod), 0o644)
		t.Logf("  module written to %s", p)
	}
}
