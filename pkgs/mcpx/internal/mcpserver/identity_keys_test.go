package mcpserver_test

import (
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/mcpserver"
)

// #1932 remainder: the _meta session keys are a configured list whose
// default also reads the key OpenCode v2 sends, so opencode sessions no
// longer share one scope on the daemon.
func TestSessionMetaKeys(t *testing.T) {
	newServer := func() (*mcpserver.Server, *identityBackend) {
		b := &identityBackend{fakeBackend: newBackend()}
		return mcpserver.New(b, "mcpx", "test"), b
	}
	call := func(t *testing.T, s *mcpserver.Server, meta map[string]any) {
		t.Helper()
		postMCP(t, s, "", mcpserver.Request(1, "tools/call", echoCallParams(meta)))
	}

	t.Run("opencode key alone names the session", func(t *testing.T) {
		s, b := newServer()
		meta := modernParams(nil)["_meta"].(map[string]any)
		meta[mcpserver.MetaOpenCodeSession] = "ses_opencode-1"
		call(t, s, meta)
		if id := b.last(t); id.Source != mcpserver.IdentityClient || id.Key != "ses_opencode-1" {
			t.Fatalf("got %+v", id)
		}
	})

	t.Run("both keys sent: the first listed key wins", func(t *testing.T) {
		s, b := newServer()
		meta := modernParams(nil)["_meta"].(map[string]any)
		meta[mcpserver.MetaSession] = "named-by-mcpx"
		meta[mcpserver.MetaOpenCodeSession] = "ses_opencode-1"
		call(t, s, meta)
		if id := b.last(t); id.Source != mcpserver.IdentityClient || id.Key != "named-by-mcpx" {
			t.Fatalf("got %+v", id)
		}
	})

	t.Run("configured order reverses the winner", func(t *testing.T) {
		s, b := newServer()
		s.SessionMetaKeys = func() []string {
			return []string{mcpserver.MetaOpenCodeSession, mcpserver.MetaSession}
		}
		meta := modernParams(nil)["_meta"].(map[string]any)
		meta[mcpserver.MetaSession] = "named-by-mcpx"
		meta[mcpserver.MetaOpenCodeSession] = "ses_opencode-1"
		call(t, s, meta)
		if id := b.last(t); id.Source != mcpserver.IdentityClient || id.Key != "ses_opencode-1" {
			t.Fatalf("got %+v", id)
		}
	})

	t.Run("empty value is ignored, next key is read", func(t *testing.T) {
		s, b := newServer()
		meta := modernParams(nil)["_meta"].(map[string]any)
		meta[mcpserver.MetaSession] = ""
		meta[mcpserver.MetaOpenCodeSession] = "ses_opencode-2"
		call(t, s, meta)
		if id := b.last(t); id.Source != mcpserver.IdentityClient || id.Key != "ses_opencode-2" {
			t.Fatalf("got %+v", id)
		}
	})

	t.Run("oversized value is used verbatim", func(t *testing.T) {
		s, b := newServer()
		long := strings.Repeat("x", 512)
		meta := modernParams(nil)["_meta"].(map[string]any)
		meta[mcpserver.MetaOpenCodeSession] = long
		call(t, s, meta)
		if id := b.last(t); id.Source != mcpserver.IdentityClient || id.Key != long {
			t.Fatalf("got source %q key length %d", id.Source, len(id.Key))
		}
	})

	t.Run("no key: the unnamed-request fallback is unchanged", func(t *testing.T) {
		s, b := newServer()
		call(t, s, modernParams(nil)["_meta"].(map[string]any))
		if id := b.last(t); id.Source != mcpserver.IdentityNone || id.Key != "" {
			t.Fatalf("got %+v", id)
		}
	})
}
