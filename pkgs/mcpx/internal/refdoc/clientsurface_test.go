package refdoc_test

import (
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/refdoc"
)

// surface renders one synthetic source file the way the committed
// public-client region is rendered.
func surface(t *testing.T, src string) string {
	t.Helper()
	out, err := refdoc.ClientSurface(map[string][]byte{"fixture.go": []byte(src)})
	if err != nil {
		t.Fatal(err)
	}
	return out
}

const surfaceBase = `package fixture

// Client talks to the daemon.
type Client struct {
	base string
}

// Option configures a Client.
type Option func(*Client)

// WithToken sets the bearer token.
func WithToken(token string) Option { return nil }

// Call performs the named operation and decodes the answer into out.
//
// A second paragraph that is not part of the summary.
func (c *Client) Call(ctx context.Context, name string, args map[string]any, out any) error {
	return nil
}

// invoke is the unexported twin.
func invoke(s string) error { return nil }

func (c *Client) privateHelper(n int) int { return n }

func (u *unexported) AlsoHidden(n int) int { return n }

var ErrUnknownOperation = errors.New("client: unknown operation")
`

// TestClientSurfaceRendersExportedCallables pins the rendering: types,
// then functions, then methods, each with its doc summary, and nothing
// unexported.
func TestClientSurfaceRendersExportedCallables(t *testing.T) {
	got := surface(t, surfaceBase)
	want := "| signature | what it is |\n" +
		"| --- | --- |\n" +
		"| `type Client struct` | Client talks to the daemon. |\n" +
		"| `type Option func(*Client)` | Option configures a Client. |\n" +
		"| `WithToken(token string) Option` | WithToken sets the bearer token. |\n" +
		"| `(*Client).Call(ctx context.Context, name string, args map[string]any, out any) error` | Call performs the named operation and decodes the answer into out. |\n"
	if got != want {
		t.Fatalf("rendered surface:\n%s\nwant:\n%s", got, want)
	}
	for _, hidden := range []string{"invoke", "privateHelper", "AlsoHidden", "unexported", "ErrUnknownOperation"} {
		if strings.Contains(got, hidden) {
			t.Errorf("surface names unexported or non-callable %q:\n%s", hidden, got)
		}
	}
}

// TestClientSurfaceTracksSignatureDrift is the guard for the class of
// change the reference exists to catch: any edit to an exported
// callable's receiver, parameters, results or visibility must change
// the rendered reference.
func TestClientSurfaceTracksSignatureDrift(t *testing.T) {
	base := surface(t, surfaceBase)
	cases := map[string]struct{ old, new string }{
		"parameter type":  {"ctx context.Context", "ctx string"},
		"parameter name":  {"name string", "operation string"},
		"receiver type":   {"func (c *Client) Call(", "func (c *Server) Call("},
		"result type":     {"args map[string]any, out any) error", "args map[string]any, out any) (int, error)"},
		"type parameters": {"func WithToken(token string) Option", "func WithToken[T any](token T) Option"},
		"exported func removed": {
			"// WithToken sets the bearer token.\nfunc WithToken(token string) Option { return nil }\n",
			"",
		},
		"exported method added": {
			"func invoke(s string) error { return nil }",
			"func (c *Client) Extra() {}\n\nfunc invoke(s string) error { return nil }",
		},
		"type kind changed": {"// Client talks to the daemon.\ntype Client struct {\n\tbase string\n}", "// Client talks to the daemon.\ntype Client interface {\n\tClose() error\n}"},
	}
	for name, c := range cases {
		t.Run(name, func(t *testing.T) {
			if !strings.Contains(surfaceBase, c.old) {
				t.Fatalf("fixture lost %q", c.old)
			}
			mutated := surface(t, strings.Replace(surfaceBase, c.old, c.new, 1))
			if mutated == base {
				t.Fatalf("%s did not change the rendered reference:\n%s", name, mutated)
			}
		})
	}
}

// TestClientSurfacePreservesNonSignatureEdits is the other half: an
// implementation body, a struct field, an unexported declaration and a
// package-level value are not part of the surface, so editing them must
// leave the reference byte-identical.
func TestClientSurfacePreservesNonSignatureEdits(t *testing.T) {
	base := surface(t, surfaceBase)
	cases := map[string]struct{ old, new string }{
		"exported method body":  {"return nil\n}\n\n// invoke", "return errors.New(\"x\")\n}\n\n// invoke"},
		"struct fields":         {"base string", "base string\n\ttoken string\n\thc any"},
		"unexported func body":  {"func invoke(s string) error { return nil }", "func invoke(s string) error { return errors.New(s) }"},
		"unexported method":     {"func (c *Client) privateHelper(n int) int { return n }", "func (c *Client) privateHelper(n string) string { return n }"},
		"package-level var":     {"var ErrUnknownOperation = errors.New(\"client: unknown operation\")", "var ErrUnknownOperation = errors.New(\"client: nope\")"},
		"method on hidden type": {"func (u *unexported) AlsoHidden(n int) int { return n }", "func (u *unexported) AlsoHidden(n string) string { return n }"},
	}
	for name, c := range cases {
		t.Run(name, func(t *testing.T) {
			if !strings.Contains(surfaceBase, c.old) {
				t.Fatalf("fixture lost %q", c.old)
			}
			mutated := surface(t, strings.Replace(surfaceBase, c.old, c.new, 1))
			if mutated != base {
				t.Fatalf("%s changed the rendered reference:\n%s\nwant:\n%s", name, mutated, base)
			}
		})
	}
}

// TestClientSurfaceIsDeterministic proves file order cannot leak into
// the output: the same declarations split across two files, fed in
// either order, render identically.
func TestClientSurfaceIsDeterministic(t *testing.T) {
	a := "package fixture\n\n// Client talks to the daemon.\ntype Client struct{}\n"
	b := "package fixture\n\n// Call performs the named operation.\nfunc (c *Client) Call(ctx context.Context) error { return nil }\n"
	render := func(files map[string][]byte) string {
		out, err := refdoc.ClientSurface(files)
		if err != nil {
			t.Fatal(err)
		}
		return out
	}
	one := render(map[string][]byte{"a.go": []byte(a), "b.go": []byte(b)})
	two := render(map[string][]byte{"b.go": []byte(b), "a.go": []byte(a)})
	if one != two || one == "" {
		t.Fatalf("render depends on map order:\n%q\n%q", one, two)
	}
}
