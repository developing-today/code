package mcpclient_test

import (
	"context"
	"encoding/json"
	"sync"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
)

// initializeParamsWithRoot connects once with the given instance root and
// returns the params of the initialize request it sent, or nil if initialize
// was answered by the modern path instead.
func initializeParamsWithRoot(t *testing.T, root string) map[string]any {
	t.Helper()
	f := newFake("legacy")
	var (
		mu    sync.Mutex
		init  map[string]any
		first bool
	)
	f.onSend = func(msg []byte) {
		var frame struct {
			Method string          `json:"method"`
			Params json.RawMessage `json:"params"`
		}
		if json.Unmarshal(msg, &frame) != nil || frame.Method != "initialize" {
			return
		}
		mu.Lock()
		defer mu.Unlock()
		if first {
			return
		}
		first = true
		_ = json.Unmarshal(frame.Params, &init)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	c, err := mcpclient.NewWithOptions(ctx, f, mcpclient.Options{
		ClientName:    "mcpx",
		ClientVersion: "test",
		Preference:    mcpclient.PreferLegacy,
		Root:          root,
	})
	if err != nil {
		t.Fatalf("connect: %v", err)
	}
	defer c.Close()
	mu.Lock()
	defer mu.Unlock()
	return init
}

// A scope-resolved instance names its workspace at initialize, in both the
// params and the declared capability, so a workspace server (gopls) opens a
// real workspace instead of loading files ad hoc (decision 0004 §5).
func TestInitializeNamesTheScopeResolvedRoot(t *testing.T) {
	params := initializeParamsWithRoot(t, "/home/user/git/code")
	if params == nil {
		t.Fatal("no initialize params captured")
	}
	if got, want := params["rootUri"], "file:///home/user/git/code"; got != want {
		t.Errorf("rootUri = %v, want %v", got, want)
	}
	folders, ok := params["workspaceFolders"].([]any)
	if !ok || len(folders) != 1 {
		t.Fatalf("workspaceFolders = %v, want one folder", params["workspaceFolders"])
	}
	folder, _ := folders[0].(map[string]any)
	if folder["uri"] != "file:///home/user/git/code" {
		t.Errorf("workspaceFolders[0].uri = %v", folder["uri"])
	}
	if folder["name"] != "code" {
		t.Errorf("workspaceFolders[0].name = %v, want the base name of the root", folder["name"])
	}

	caps, _ := params["capabilities"].(map[string]any)
	workspace, _ := caps["workspace"].(map[string]any)
	if workspace["workspaceFolders"] != true {
		t.Errorf("capabilities.workspace = %v, want workspaceFolders declared true", caps["workspace"])
	}
}

// A global-scoped (or otherwise rootless) instance sends no workspace fields
// and declares no workspace capability: no fabricated daemon-cwd root.
func TestInitializeWithoutARootIsUnchanged(t *testing.T) {
	params := initializeParamsWithRoot(t, "")
	if params == nil {
		t.Fatal("no initialize params captured")
	}
	for _, key := range []string{"rootUri", "workspaceFolders"} {
		if v, ok := params[key]; ok {
			t.Errorf("%s = %v, want it absent for a rootless instance", key, v)
		}
	}
	caps, _ := params["capabilities"].(map[string]any)
	if v, ok := caps["workspace"]; ok {
		t.Errorf("capabilities.workspace = %v, want it absent for a rootless instance", v)
	}
}
