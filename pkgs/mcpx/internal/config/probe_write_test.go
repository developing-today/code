package config_test

import (
	"os"
	"testing"

	"github.com/dezren39/mcpx/internal/codegen"
)

func TestProbeWriteModule(t *testing.T) {
	mod := codegen.Module([]codegen.Namespace{{Name: "my-ns", Server: "a", Tools: []codegen.Tool{{Name: "ping"}}}}, "", "")
	os.WriteFile(os.Getenv("PROBE_OUT"), []byte(mod), 0o644)
}
