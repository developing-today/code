package config

import (
	"encoding/json"
	"reflect"
	"strings"
	"testing"
)

// #1933 slice 4: the generator emits tool-view and virtual-namespace
// declarations for a composed tool set. The oracles are named after
// what they pin down: the emitted shape is the hand-written shape,
// and a generated declaration resolves to the same catalogue as the
// equivalent hand-written one.

func composedToolSet() []ViewSpec {
	return []ViewSpec{
		{
			Name:        "tailscale-peek",
			AliasOf:     "tailscale",
			Description: "read-only tailnet listing",
			Tools: []ViewToolSpec{
				{Name: "listTailnet", From: "list_tailnets"},
				{Name: "listHostnames", From: "list_devices",
					Pin:         map[string]any{"tailnet": "-"},
					Hide:        []string{"tailnet"},
					Description: "hostnames on the current tailnet"},
			},
		},
		{
			Name:        "combo",
			Description: "echoes across servers",
			Tools: []ViewToolSpec{
				{Name: "alphaEcho", From: "alpha.echo", Pin: map[string]any{"fixed": "A"}, Hide: []string{"fixed"}},
				{Name: "betaEcho", From: "beta.echo"},
			},
		},
	}
}

// The hand-written declaration of the same composed tool set: the
// targets plus both views, as an operator would write them.
const handWrittenOracle = `{
  "mcpServers": {
    "tailscale": {"command": "tailscale-mcp"},
    "alpha": {"command": "alpha-bin"},
    "beta": {"url": "http://127.0.0.1:9/beta"},
    "tailscale-peek": {
      "aliasOf": "tailscale",
      "mcpx": {
        "description": "read-only tailnet listing",
        "toolViews": {
          "listTailnet": {"from": "list_tailnets"},
          "listHostnames": {
            "from": "list_devices",
            "pin": {"tailnet": "-"},
            "hide": ["tailnet"],
            "description": "hostnames on the current tailnet"
          }
        }
      }
    },
    "combo": {
      "mcpx": {
        "description": "echoes across servers",
        "toolViews": {
          "alphaEcho": {"from": "alpha.echo", "pin": {"fixed": "A"}, "hide": ["fixed"]},
          "betaEcho": {"from": "beta.echo"}
        }
      }
    }
  }
}`

const targetsOracle = `{
  "mcpServers": {
    "tailscale": {"command": "tailscale-mcp"},
    "alpha": {"command": "alpha-bin"},
    "beta": {"url": "http://127.0.0.1:9/beta"}
  }
}`

// mergeFragments stands in for the operator merging the generated
// fragment into their config next to the target servers: the two
// mcpServers maps are unioned and the result is parsed whole.
func mergeFragments(t *testing.T, base, fragment []byte) *Config {
	t.Helper()
	var b, f struct {
		MCPServers map[string]json.RawMessage `json:"mcpServers"`
	}
	if err := json.Unmarshal(base, &b); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(fragment, &f); err != nil {
		t.Fatal(err)
	}
	for name, raw := range f.MCPServers {
		if _, clash := b.MCPServers[name]; clash {
			t.Fatalf("generated fragment redeclares %q", name)
		}
		b.MCPServers[name] = raw
	}
	merged, err := json.Marshal(struct {
		MCPServers map[string]json.RawMessage `json:"mcpServers"`
	}{MCPServers: b.MCPServers})
	if err != nil {
		t.Fatal(err)
	}
	c, err := parse(merged)
	if err != nil {
		t.Fatal(err)
	}
	return c
}

func TestGeneratedViewsMatchHandWrittenShape(t *testing.T) {
	fragment, err := GenerateViewsJSON(composedToolSet())
	if err != nil {
		t.Fatal(err)
	}
	got, err := parse(fragment)
	if err != nil {
		t.Fatal(err)
	}
	want, err := parse([]byte(handWrittenOracle))
	if err != nil {
		t.Fatal(err)
	}
	for _, name := range GeneratedViewNames(composedToolSet()) {
		g, w := got.MCPServers[name], want.MCPServers[name]
		if g == nil || w == nil {
			t.Fatalf("view %q: generated %v, hand-written %v", name, g != nil, w != nil)
		}
		if g.AliasOf != w.AliasOf {
			t.Errorf("view %q: aliasOf got %q, want %q", name, g.AliasOf, w.AliasOf)
		}
		if !reflect.DeepEqual(g.Mcpx, w.Mcpx) {
			t.Errorf("view %q: emitted mcpx block differs from the hand-written one:\n got %+v\nwant %+v", name, g.Mcpx, w.Mcpx)
		}
	}
	// Deterministic: generating twice is byte-identical.
	again, err := GenerateViewsJSON(composedToolSet())
	if err != nil {
		t.Fatal(err)
	}
	if string(again) != string(fragment) {
		t.Errorf("generation is not deterministic:\n%s\n---\n%s", fragment, again)
	}
}

func TestGeneratedViewsRoundTripThroughResolve(t *testing.T) {
	fragment, err := GenerateViewsJSON(composedToolSet())
	if err != nil {
		t.Fatal(err)
	}
	generated := mergeFragments(t, []byte(targetsOracle), fragment)
	handWritten, err := parse([]byte(handWrittenOracle))
	if err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"tailscale-peek", "combo"} {
		g, err := generated.Resolve(name)
		if err != nil {
			t.Fatalf("generated %q: %v", name, err)
		}
		w, err := handWritten.Resolve(name)
		if err != nil {
			t.Fatalf("hand-written %q: %v", name, err)
		}
		if g.Virtual() != w.Virtual() {
			t.Errorf("%q: virtual got %v, want %v", name, g.Virtual(), w.Virtual())
		}
		if !reflect.DeepEqual(g.ToolViews, w.ToolViews) {
			t.Errorf("%q: resolved catalogue differs:\n got %+v\nwant %+v", name, g.ToolViews, w.ToolViews)
		}
	}
	// The virtual namespace's mounts split exactly as the
	// hand-written one's do.
	combo, err := generated.Resolve("combo")
	if err != nil {
		t.Fatal(err)
	}
	if tv := combo.ToolViews["alphaEcho"]; tv.MountServer != "alpha" || tv.MountTool != "echo" {
		t.Errorf("generated mount split: got %q/%q, want alpha/echo", tv.MountServer, tv.MountTool)
	}
}

func TestGenerateViewsRefusesUndeclarableCompositions(t *testing.T) {
	cases := map[string][]ViewSpec{
		"view with no name":  {{Tools: []ViewToolSpec{{Name: "x", From: "a.b"}}}},
		"view with no tools": {{Name: "v"}},
		"duplicate view": {
			{Name: "v", Tools: []ViewToolSpec{{Name: "x", From: "a.b"}}},
			{Name: "v", Tools: []ViewToolSpec{{Name: "y", From: "a.b"}}},
		},
		"duplicate tool in a view": {{Name: "v", Tools: []ViewToolSpec{
			{Name: "x", From: "a.b"}, {Name: "x", From: "a.c"}}}},
		"tool with no source": {{Name: "v", Tools: []ViewToolSpec{{Name: "x"}}}},
		"tool with two sources": {{Name: "v", Tools: []ViewToolSpec{
			{Name: "x", From: "a.b", Command: "run.sh"}}}},
		"bare from on a virtual namespace": {{Name: "v", Tools: []ViewToolSpec{
			{Name: "x", From: "echo"}}}},
		"hide without pin": {{Name: "v", AliasOf: "a", Tools: []ViewToolSpec{
			{Name: "x", From: "t", Hide: []string{"secret"}}}}},
		"command view without description": {{Name: "v", Tools: []ViewToolSpec{
			{Name: "x", Command: "run.sh"}}}},
	}
	for name, views := range cases {
		if _, err := GenerateViewsJSON(views); err == nil {
			t.Errorf("%s: generation must refuse, got nil error", name)
		} else if !strings.Contains(err.Error(), "generate views") {
			t.Errorf("%s: error must name the generator, got %v", name, err)
		}
	}
}

func TestGeneratedScriptViewRoundTrips(t *testing.T) {
	fragment, err := GenerateViewsJSON([]ViewSpec{{
		Name: "ops",
		Tools: []ViewToolSpec{
			{Name: "shout", Command: "shout.sh", Description: "shout the input", ReadOnly: true},
		},
	}})
	if err != nil {
		t.Fatal(err)
	}
	c := mergeFragments(t, []byte(targetsOracle), fragment)
	r, err := c.Resolve("ops")
	if err != nil {
		t.Fatal(err)
	}
	tv, ok := r.ToolViews["shout"]
	if !ok {
		t.Fatalf("generated script view missing from the resolved catalogue: %+v", r.ToolViews)
	}
	if tv.Command != "shout.sh" || !tv.ReadOnly {
		t.Errorf("script view resolved wrong: %+v", tv)
	}
}
