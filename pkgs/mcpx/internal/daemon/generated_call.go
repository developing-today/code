package daemon

import (
	"context"
	"encoding/json"
	"fmt"
	"sort"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/codegen"
	"github.com/developing-today/code/pkgs/mcpx/internal/config"
	"github.com/developing-today/code/pkgs/mcpx/internal/mcpclient"
	"github.com/developing-today/code/pkgs/mcpx/internal/pool"
)

// generatedScopes preserves a caller's choice BEFORE a schema refresh creates
// an instance. Re-disambiguating afterwards could mistake that new instance
// for a previous choice. This context is private to this single call.
type generatedScopesKey struct{}
type generatedScopes map[*pool.Pool]config.ResolvedScope
type generatedSchemasKey struct{}
type generatedSchemas map[*pool.Pool][]mcpclient.Tool
type generatedTargetSnapshot struct {
	pool *pool.Pool
	view *config.Resolved
}
type generatedTargetsKey struct{}
type generatedTargets map[string]generatedTargetSnapshot

// policySchemas pins annotation decisions to the exact caller-scoped schema
// used to resolve this generated call, not another caller's later cache write.
// Ordinary calls retain their existing cache and expiry behavior.
func policySchemas(ctx context.Context, p *pool.Pool) ([]mcpclient.Tool, time.Time, bool) {
	if snapshots, ok := ctx.Value(generatedSchemasKey{}).(generatedSchemas); ok {
		if tools, found := snapshots[p]; found {
			return tools, time.Now(), false
		}
	}
	tools, _, _, at, deadline, _ := p.CachedAll()
	return tools, at, !at.IsZero() && !deadline.IsZero() && !time.Now().Before(deadline)
}

func generatedScope(ctx context.Context, p *pool.Pool) (config.ResolvedScope, bool) {
	scopes, _ := ctx.Value(generatedScopesKey{}).(generatedScopes)
	rs, ok := scopes[p]
	return rs, ok
}

// generatedName resolves only advertised names, never guessed raw spellings.
// Collisions are refused even if one spelling happens to match the property.
func generatedName(language, property string, names []string) (string, error) {
	var matches []string
	for _, name := range names {
		normalized, valid := codegen.GeneratedToolName(language, name)
		if !valid {
			return "", fmt.Errorf("unsupported generated language %q", language)
		}
		if normalized == property {
			matches = append(matches, name)
		}
	}
	sort.Strings(matches)
	if len(matches) != 1 {
		if len(matches) > 1 {
			return "", fmt.Errorf("generated member %q is ambiguous: %v", property, matches)
		}
		return "", fmt.Errorf("unknown generated member %q", property)
	}
	return matches[0], nil
}

// generatedTarget takes one consistent view/pool snapshot across reloads.
func (r *Registry) generatedTarget(name string) (*pool.Pool, *config.Resolved, bool) {
	r.mu.RLock()
	defer r.mu.RUnlock()
	if p, ok := r.pools[name]; ok {
		v := r.views[name]
		return p, v, v != nil
	}
	for _, key := range r.order {
		if v := r.views[key]; v != nil && v.Namespace == name {
			return r.pools[key], v, r.pools[key] != nil
		}
	}
	return nil, nil, false
}

func (r *Registry) generatedCatalogue(ctx context.Context, p *pool.Pool, view *config.Resolved, server string, cc config.CallContext) ([]mcpclient.Tool, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if err := r.refuseBlocked(server); err != nil {
		return nil, err
	}
	ctx.Value(generatedTargetsKey{}).(generatedTargets)[server] = generatedTargetSnapshot{p, view}
	all, _, _, at, deadline, private := p.CachedAll()
	if view.ScriptOnly() {
		return visibleTools(view, all), nil
	}
	scopes := ctx.Value(generatedScopesKey{}).(generatedScopes)
	rs, ok := scopes[p]
	if !ok {
		rs = r.disambiguate(ctx, p, cc, r.resolvedFor(p, cc))
		scopes[p] = rs
	}
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if at.IsZero() || (!deadline.IsZero() && !time.Now().Before(deadline)) || p.CallerScoped() || private {
		var err error
		all, _, err = p.RefreshSchemasFor(ctx, rs)
		if err != nil {
			return nil, err
		}
	}
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	ctx.Value(generatedSchemasKey{}).(generatedSchemas)[p] = all
	return visibleTools(view, all), nil
}

// CallGenerated is opt-in for members of a cold generated namespace. The
// authoritative schema is fetched in the caller's scope, then ordinary call
// dispatch still enforces views, pins, argument rules, capabilities and policy.
// Existing raw clients and eager startup behavior are deliberately unchanged.
func (r *Registry) CallGenerated(ctx context.Context, reportingID, language, server, property string, cc config.CallContext, args any, selectors ...string) (json.RawMessage, error) {
	if _, valid := codegen.GeneratedToolName(language, ""); !valid {
		return nil, fmt.Errorf("unsupported generated language %q", language)
	}
	ctx = context.WithValue(ctx, generatedScopesKey{}, generatedScopes{})
	ctx = context.WithValue(ctx, generatedSchemasKey{}, generatedSchemas{})
	targets := generatedTargets{}
	ctx = context.WithValue(ctx, generatedTargetsKey{}, targets)
	p, view, ok := r.generatedTarget(server)
	if !ok {
		return nil, UnknownServer{Name: server}
	}
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if err := r.refuseBlocked(server); err != nil {
		return nil, err
	}
	targets[server] = generatedTargetSnapshot{p, view}
	var names []string
	if view.Virtual() {
		// Virtual names are declared by config, including those whose source is
		// still cold. Check all declarations for collisions without starting every
		// mounted server just to invoke one member.
		for name := range view.ToolViews {
			if view.VisibleTool(name) {
				names = append(names, name)
			}
		}
	} else {
		all, err := r.generatedCatalogue(ctx, p, view, server, cc)
		if err != nil {
			return nil, err
		}
		for _, t := range all {
			names = append(names, t.Name)
		}
	}
	if len(selectors) > 0 && selectors[0] != "" {
		selected := names[:0]
		for _, name := range names {
			if name == selectors[0] || codegen.ToolFuncName(name) == selectors[0] {
				selected = append(selected, name)
			}
		}
		names = selected
	}
	tool, err := generatedName(language, property, names)
	if err != nil {
		return nil, err
	}
	if view.Virtual() {
		tv := view.ToolViews[tool]
		if tv.MountServer != "" {
			target, targetView, exists := r.generatedTarget(tv.MountServer)
			if !exists {
				return nil, UnknownServer{Name: tv.MountServer}
			}
			all, err := r.generatedCatalogue(ctx, target, targetView, tv.MountServer, cc)
			if err != nil {
				return nil, err
			}
			found := false
			for _, t := range all {
				if t.Name == tv.MountTool {
					found = true
					break
				}
			}
			if !found {
				return nil, HiddenTool{Server: tv.MountServer, Tool: tv.MountTool}
			}
		}
	}
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	for name, old := range targets {
		current, currentView, exists := r.generatedTarget(name)
		if !exists || current != old.pool || currentView != old.view {
			return nil, fmt.Errorf("server %q configuration changed while resolving generated member; retry", name)
		}
	}
	if reportingID != "" {
		return r.CallReporting(ctx, reportingID, server, tool, cc, args)
	}
	return r.Call(ctx, server, tool, cc, args)
}
