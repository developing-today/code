package config

import (
	"encoding/json"
	"fmt"
	"sort"
	"strings"
)

// #1933 slice 4: generators that emit declarations. Slices 1-3 made
// tool views declarable by hand; this is the deterministic author-time
// generator Approach C of #1933 asks for on top: a composed tool set
// goes in, a declaration in exactly the hand-written config shape
// comes out, and the daemon at runtime reads only the declaration,
// exactly as it does today. Nothing here runs at daemon time.

// ViewToolSpec names one tool of a composed tool set: the name the
// view exposes, and where the tool comes from -- a bare tool name on
// the aliased server (alias view), a qualified "server.tool" mount
// (virtual namespace), or a local script (Command or File).
type ViewToolSpec struct {
	Name        string
	From        string
	Pin         map[string]any
	Hide        []string
	Description string
	Command     string
	Args        []string
	File        string
	Timeout     string
	ReadOnly    bool
}

// ViewSpec is one namespace of a composed tool set to declare: an
// alias view when AliasOf names the target server, a virtual
// namespace when it is empty.
type ViewSpec struct {
	Name        string
	AliasOf     string
	Description string
	Tools       []ViewToolSpec
}

// GenerateViewServers renders composed tool sets as server
// declarations, in the same structs a hand-written config parses
// into. The composition is checked as it is rendered: a generator
// that emits a declaration the loader would refuse has failed at its
// one job, so the structural rules the loader enforces without
// knowing the targets (one source per view, hide only removes
// pinned properties, a mount must be qualified) are enforced here
// too, with the view and tool named.
func GenerateViewServers(views []ViewSpec) (map[string]*Server, error) {
	out := make(map[string]*Server, len(views))
	for _, v := range views {
		if v.Name == "" {
			return nil, fmt.Errorf("generate views: a view's name must not be empty")
		}
		if _, dup := out[v.Name]; dup {
			return nil, fmt.Errorf("generate views: view %q is declared twice", v.Name)
		}
		if len(v.Tools) == 0 {
			return nil, fmt.Errorf("generate views: view %q declares no tools", v.Name)
		}
		tvs := make(map[string]ToolView, len(v.Tools))
		for _, t := range v.Tools {
			if t.Name == "" {
				return nil, fmt.Errorf("generate views: view %q: a tool's name must not be empty", v.Name)
			}
			if _, dup := tvs[t.Name]; dup {
				return nil, fmt.Errorf("generate views: view %q: tool %q is declared twice", v.Name, t.Name)
			}
			tv, err := generateToolView(v, t)
			if err != nil {
				return nil, err
			}
			tvs[t.Name] = tv
		}
		out[v.Name] = &Server{
			Name:    v.Name,
			AliasOf: v.AliasOf,
			Mcpx:    &Extras{Description: v.Description, ToolViews: tvs},
		}
	}
	return out, nil
}

func generateToolView(v ViewSpec, t ViewToolSpec) (ToolView, error) {
	sources := 0
	for _, s := range []string{t.From, t.Command, t.File} {
		if s != "" {
			sources++
		}
	}
	if sources != 1 {
		return ToolView{}, fmt.Errorf("generate views: view %q: tool %q: exactly one of from, command or file is required", v.Name, t.Name)
	}
	if t.From != "" && v.AliasOf == "" && !strings.Contains(t.From, ".") {
		return ToolView{}, fmt.Errorf("generate views: view %q: tool %q: from %q names no server; a virtual namespace mounts by qualified server.tool name", v.Name, t.Name, t.From)
	}
	for _, h := range t.Hide {
		if _, pinned := t.Pin[h]; !pinned {
			return ToolView{}, fmt.Errorf("generate views: view %q: tool %q: hide lists %q, which is not pinned; hide removes pinned properties", v.Name, t.Name, h)
		}
	}
	if t.Command != "" && t.Description == "" {
		return ToolView{}, fmt.Errorf("generate views: view %q: tool %q: description is required for a command-backed view; callers choose a tool by it", v.Name, t.Name)
	}
	return ToolView{
		From:        t.From,
		Pin:         t.Pin,
		Hide:        t.Hide,
		Description: t.Description,
		Command:     t.Command,
		Args:        t.Args,
		File:        t.File,
		Timeout:     t.Timeout,
		ReadOnly:    t.ReadOnly,
	}, nil
}

// GenerateViewsJSON renders composed tool sets as a config fragment
// -- {"mcpServers": {...}} -- ready to merge into a config file or
// hand to the ordinary loader. Rendering goes through the same
// structs parsing produces and encoding/json emits map keys in
// sorted order, so the output is deterministic and shaped exactly
// like a hand-written declaration.
func GenerateViewsJSON(views []ViewSpec) ([]byte, error) {
	servers, err := GenerateViewServers(views)
	if err != nil {
		return nil, err
	}
	fragment := struct {
		MCPServers map[string]*Server `json:"mcpServers"`
	}{MCPServers: servers}
	return json.MarshalIndent(fragment, "", "  ")
}

// GeneratedViewNames is the sorted set of namespace names a
// composition declares, for callers presenting what a generation
// would write before they write it.
func GeneratedViewNames(views []ViewSpec) []string {
	names := make([]string, 0, len(views))
	for _, v := range views {
		names = append(names, v.Name)
	}
	sort.Strings(names)
	return names
}
