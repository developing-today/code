package gen

import (
	"encoding/json"
	"fmt"
	"sort"
	"strings"
)

// The first two registered generators (#1958). Each draw is a
// pure function of the Source stream: no map iteration order, no
// time, no environment — the same stream always draws the same
// input, which is the whole point of the registry.

// --- config.layers -------------------------------------------------

// LayerServer is one server entry in one config layer.
type LayerServer struct {
	Name    string `json:"name"`
	Command string `json:"command"`
}

// ConfigLayer is one .mcpx.json layer: a set of servers, and
// whether its rendered file carries a JSONC comment.
type ConfigLayer struct {
	Servers []LayerServer `json:"servers"`
	Comment bool          `json:"comment"`
}

// ConfigLayers is a stack of config layers, farthest first,
// nearest last — the order LoadFrom's chain walk merges them in
// (nearest wins per server).
type ConfigLayers struct {
	Layers []ConfigLayer `json:"layers"`
}

var configLayerServerNames = []string{"alpha", "beta", "gamma", "delta"}

// DrawConfigLayers draws a stack of 1–4 layers over a small server
// alphabet, so layers overlap on server names often enough for
// nearest-wins to be exercised, and far layers name servers the
// near ones never mention. size bounds the layer count the same
// way gen.Sizes grows inputs: bigger size, deeper stack.
func DrawConfigLayers(src Source, size int) ConfigLayers {
	if size < 0 {
		size = 0
	}
	maxLayers := 1 + size
	if maxLayers > 4 {
		maxLayers = 4
	}
	n := 1 + Intn(src, maxLayers)
	out := ConfigLayers{Layers: make([]ConfigLayer, 0, n)}
	for li := 0; li < n; li++ {
		layer := ConfigLayer{Comment: Intn(src, 4) == 0}
		// A shuffled prefix of the alphabet: each server at most
		// once per layer, overlap across layers by construction.
		names := append([]string{}, configLayerServerNames...)
		for i := len(names) - 1; i > 0; i-- {
			j := Intn(src, i+1)
			names[i], names[j] = names[j], names[i]
		}
		k := 1 + Intn(src, len(names))
		for _, name := range names[:k] {
			layer.Servers = append(layer.Servers, LayerServer{
				Name:    name,
				Command: fmt.Sprintf("cmd-%s-L%d-%d", name, li, Intn(src, 1000)),
			})
		}
		sort.Slice(layer.Servers, func(a, b int) bool {
			return layer.Servers[a].Name < layer.Servers[b].Name
		})
		out.Layers = append(out.Layers, layer)
	}
	return out
}

// Winner returns the command the nearest layer gives the named
// server, and whether any layer names it — the value a correct
// merge must produce.
func (c ConfigLayers) Winner(name string) (string, bool) {
	for li := len(c.Layers) - 1; li >= 0; li-- {
		for _, s := range c.Layers[li].Servers {
			if s.Name == name {
				return s.Command, true
			}
		}
	}
	return "", false
}

// Names returns every server name any layer mentions, sorted.
func (c ConfigLayers) Names() []string {
	seen := map[string]bool{}
	var out []string
	for _, l := range c.Layers {
		for _, s := range l.Servers {
			if !seen[s.Name] {
				seen[s.Name] = true
				out = append(out, s.Name)
			}
		}
	}
	sort.Strings(out)
	return out
}

// Render returns the layer files' texts, farthest first. With
// comments, a layer whose Comment flag is set carries a leading
// JSONC line comment; without, the text is plain JSON (the
// StripJSONC identity case).
func (c ConfigLayers) Render(comments bool) []string {
	out := make([]string, 0, len(c.Layers))
	for _, l := range c.Layers {
		servers := map[string]any{}
		for _, s := range l.Servers {
			servers[s.Name] = map[string]any{"command": s.Command}
		}
		b, _ := json.Marshal(map[string]any{"mcpServers": servers})
		text := string(b)
		if comments && l.Comment {
			text = "// generated layer\n" + text
		}
		out = append(out, text)
	}
	return out
}

// Canonical is config.layers' registry Draw form.
func (c ConfigLayers) Canonical() string {
	b, _ := json.Marshal(c)
	return string(b)
}

// --- launcher.template ---------------------------------------------

// LauncherTemplate is one drawn template plus the fill that
// expands it. Segments alternate literal / placeholder, starting
// and ending with a (possibly empty) literal; literals never
// start or end with a placeholder-name character where they touch
// a placeholder (or with '(' — @name( reads as an argument
// list), so the placeholder parses exactly as drawn. Fill
// values contain no '@', so expansion cannot recurse or leave a
// placeholder behind.
type LauncherTemplate struct {
	Segments []string          `json:"segments"` // even indexes: literals; odd: placeholder names
	Fill     map[string]string `json:"fill"`
}

var launcherPlaceholderNames = []string{
	"header", "globals", "console", "before", "prefix",
	"import", "entry", "onSuccess", "onError", "suffix",
}

var launcherLiterals = []string{
	"", " ", "\n", "; ", " { ", " } ", "= x ", " (); ", " -- ", "\n\n",
}

// DrawLauncherTemplate draws a template using each placeholder at
// most once (a repeat use is an expansion error unless explicitly
// allowed, which is a different shape), with placeholders
// sometimes directly adjacent.
func DrawLauncherTemplate(src Source, size int) LauncherTemplate {
	if size < 0 {
		size = 0
	}
	names := append([]string{}, launcherPlaceholderNames...)
	for i := len(names) - 1; i > 0; i-- {
		j := Intn(src, i+1)
		names[i], names[j] = names[j], names[i]
	}
	maxUse := 1 + size
	if maxUse > len(names) {
		maxUse = len(names)
	}
	k := 1 + Intn(src, maxUse)
	used := names[:k]
	out := LauncherTemplate{Fill: map[string]string{}}
	var segs []string
	segs = append(segs, launcherLiterals[Intn(src, len(launcherLiterals))])
	for _, name := range used {
		segs = append(segs, name)
		segs = append(segs, launcherLiterals[Intn(src, len(launcherLiterals))])
		out.Fill[name] = fmt.Sprintf("<<%s:%d>>", name, Intn(src, 1000))
	}
	out.Segments = segs
	return out
}

// Text is the template source: literals and @placeholders joined.
func (l LauncherTemplate) Text() string {
	var b strings.Builder
	for i, s := range l.Segments {
		if i%2 == 1 {
			b.WriteByte('@')
		}
		b.WriteString(s)
	}
	return b.String()
}

// Expected is the exact expansion when every used placeholder is
// filled: literals preserved byte-for-byte, each placeholder
// replaced by its fill, in order.
func (l LauncherTemplate) Expected() string {
	var b strings.Builder
	for i, s := range l.Segments {
		if i%2 == 1 {
			b.WriteString(l.Fill[s])
		} else {
			b.WriteString(s)
		}
	}
	return b.String()
}

// Canonical is launcher.template's registry Draw form.
func (l LauncherTemplate) Canonical() string {
	b, _ := json.Marshal(l)
	return string(b)
}
