package mcpserver

import (
	"context"
	"fmt"
	"sort"
)

// Raw tools on request: ?codemode=false on the /mcp URL.
//
// opencode v2 appends codemode=false to the URL of a remote MCP server
// whose configuration does not opt out of code mode, on the theory that
// "servers that bundle their own Code Mode expose raw tools when asked"
// (register PLG-35, #1940). mcpx bundles code mode -- its /mcp surface is
// ten meta-tools and a TypeScript interpreter -- so a client that asks
// gets the opposite surface instead: every upstream tool, flat, under a
// namespaced name (<namespace>_<tool>, the Native spelling the registry
// already projects), callable through the same Backend.Call a
// pass-through call takes. The gateway's own tools are not on that
// surface: the client asked for raw tools instead of the code-mode
// surface, not in addition to it.
//
// The parameter lives on the URL, so it is a property of one HTTP
// request, not of the server or the session: ServeHTTP reads it and
// every later request decides for itself. Absence -- and any value other
// than exactly "false" -- changes nothing. stdio has no URL and never
// carries it.

// codemodeKey carries the request's codemode answer.
type codemodeKey struct{}

// withCodemode records whether this request arrived at /mcp with
// ?codemode=false.
func withCodemode(ctx context.Context, off bool) context.Context {
	if !off {
		return ctx
	}
	return context.WithValue(ctx, codemodeKey{}, true)
}

// rawToolsRequested reports whether this request asked for raw tools.
func rawToolsRequested(ctx context.Context) bool {
	off, _ := ctx.Value(codemodeKey{}).(bool)
	return off
}

// NamespacedTool is one upstream tool under its owner's name, before the
// two are joined into the exposed name.
type NamespacedTool struct {
	Namespace string
	Tool      Tool // Tool.Name is the name the upstream itself uses
}

// RawToolsBackend is a Backend that can list every upstream's tools at
// once, for the raw surface. Pass-through lists one named upstream
// (PassthroughBackend); the raw surface needs all of them, and a backend
// that cannot list them answers tools/list with an error rather than a
// partial catalogue a client would trust.
type RawToolsBackend interface {
	RawTools(ctx context.Context) ([]NamespacedTool, error)
}

// rawKey carries one request's raw lookup, so the upstreams' tool lists
// are fetched once per request rather than once per question asked of
// them -- the same reason passKey exists.
type rawKey struct{}

type rawLookup struct {
	tools []Tool
	// owner maps each exposed name to the namespace and raw name that
	// serve it.
	owner map[string]NamespacedTool
	err   error
}

// withRaw resolves the raw list for the rest of this request, when the
// request asked for it.
func (s *Server) withRaw(ctx context.Context) context.Context {
	if !rawToolsRequested(ctx) {
		return ctx
	}
	if _, ok := ctx.Value(rawKey{}).(*rawLookup); ok {
		return ctx
	}
	return context.WithValue(ctx, rawKey{}, s.lookupRaw(ctx))
}

// raw is this request's raw lookup, or nil when the request did not ask
// for raw tools.
func (s *Server) raw(ctx context.Context) *rawLookup {
	if !rawToolsRequested(ctx) {
		return nil
	}
	if l, ok := ctx.Value(rawKey{}).(*rawLookup); ok {
		return l
	}
	return s.lookupRaw(ctx)
}

// lookupRaw lists every upstream through the backend and joins each
// tool's name with its namespace. Two tools joining to the same exposed
// name is refused rather than resolved, as a pass-through collision is:
// whichever won, a client reading tools/list would be calling a tool it
// was not shown.
func (s *Server) lookupRaw(ctx context.Context) *rawLookup {
	rb, ok := s.backend.(RawToolsBackend)
	if !ok {
		return &rawLookup{err: fmt.Errorf(
			"codemode=false was requested, but this backend cannot list every upstream's tools")}
	}
	all, err := rb.RawTools(ctx)
	if err != nil {
		return &rawLookup{err: fmt.Errorf("raw tools for codemode=false: %w", err)}
	}
	l := &rawLookup{owner: map[string]NamespacedTool{}}
	for _, nt := range all {
		exposed := nt.Namespace + "_" + nt.Tool.Name
		if prev, dup := l.owner[exposed]; dup {
			return &rawLookup{err: fmt.Errorf(
				"upstreams %q and %q both project to a raw tool named %q; "+
					"codemode=false cannot serve both under one name",
				prev.Namespace, nt.Namespace, exposed)}
		}
		t := nt.Tool
		t.Name = exposed
		l.owner[exposed] = nt
		l.tools = append(l.tools, t)
	}
	sort.Slice(l.tools, func(i, j int) bool { return l.tools[i].Name < l.tools[j].Name })
	return l
}

// rawSurface is the tools/list a codemode=false request gets: the raw
// tools, and nothing else.
func (s *Server) rawSurface(ctx context.Context) ([]Tool, error) {
	l := s.raw(ctx)
	if l == nil {
		return nil, nil
	}
	return l.tools, l.err
}

// rawOwner is the upstream serving an exposed raw name, or false when
// the name is not a raw tool on this request.
func (s *Server) rawOwner(ctx context.Context, name string) (NamespacedTool, bool, error) {
	l := s.raw(ctx)
	if l == nil {
		return NamespacedTool{}, false, nil
	}
	if l.err != nil {
		return NamespacedTool{}, false, l.err
	}
	nt, ok := l.owner[name]
	return nt, ok, nil
}
