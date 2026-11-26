package mcpclient

import (
	"context"
	"encoding/json"
	"os"
	"strings"
)

// Fleet context passthrough (fleet ctx propagation build 4/5, #14190).
//
// When OpenCode (or any MCP client) calls through MCPX, the fleet context
// rides to the tool call so the far side can attribute the call without any
// model effort. Where the protocol can carry headers (Streamable HTTP) the
// context travels as headers; everywhere it travels in the request's _meta.
// Where the protocol cannot carry headers (stdio) the FLEET_CTX_* env
// contract (sibling build 2/5, #14188) is the fallback: stdio servers inherit
// the parent environment, so no code is needed there beyond reading it here.
//
// Constraints (from #10357, all honored):
//   - zero model effort: captured from ctx (daemon-installed) or env, never
//     from tool arguments;
//   - cheap on the hot path: no round trip, only a map merge and header sets;
//   - no secrets in context: ids and names only, never tokens;
//   - absence is representable: no ctx and no env means no header and no
//     _meta key, not an error.
//
// Decision-record default (Q1): the env-carried record is canonical. This
// file reads it; it does not define the full variable list, which #14188
// owns. FLEET_CTX_TASK_ID (+ optional FLEET_CTX_TASK_NAME) is the queue-task
// leg this build proves; further FLEET_CTX_* keys ride the same path when
// #14188 lands them.

// FleetCtx is the queue-task leg of the fleet context record. Empty means
// absent: an interactive shell or old caller with no contract yields a
// partial record, not an error.
type FleetCtx struct {
	TaskID   string `json:"taskId,omitempty"`
	TaskName string `json:"taskName,omitempty"`
}

// Empty reports whether there is no context to carry.
func (f FleetCtx) Empty() bool {
	return f.TaskID == "" && f.TaskName == ""
}

// Env contract keys (working defaults; the full list is owned by #14188).
const (
	FleetEnvTaskID   = "FLEET_CTX_TASK_ID"
	FleetEnvTaskName = "FLEET_CTX_TASK_NAME"
)

// HTTP header carriers for Streamable HTTP, where headers can travel.
const (
	FleetHeaderTaskID   = "X-Fleet-Task-Id"
	FleetHeaderTaskName = "X-Fleet-Task-Name"
)

// _meta carriers, present on every transport (stdio included) because they
// ride the JSON-RPC body. Namespaced under dev.fleet/ like the mcpx-local
// dev.mcpx/requestState key.
const (
	FleetMetaTaskID   = "dev.fleet/taskId"
	FleetMetaTaskName = "dev.fleet/taskName"
)

type fleetCtxKey struct{}

// WithFleetCtx attaches a fleet context to the calls made under ctx.
func WithFleetCtx(ctx context.Context, f FleetCtx) context.Context {
	if f.Empty() {
		return ctx
	}
	return context.WithValue(ctx, fleetCtxKey{}, f)
}

// FleetCtxFrom returns the fleet context stored on ctx, or zero when none.
func FleetCtxFrom(ctx context.Context) FleetCtx {
	if ctx == nil {
		return FleetCtx{}
	}
	f, _ := ctx.Value(fleetCtxKey{}).(FleetCtx)
	return f
}

// FleetCtxFromEnv reads the fleet context from the process environment.
// Empty means absent (partial record, not an error).
func FleetCtxFromEnv() FleetCtx {
	return FleetCtx{
		TaskID:   strings.TrimSpace(os.Getenv(FleetEnvTaskID)),
		TaskName: strings.TrimSpace(os.Getenv(FleetEnvTaskName)),
	}
}

// fleetCtxOf resolves the effective context: explicit ctx first, env
// fallback second (the stdio/no-header path), absent when neither has it.
func fleetCtxOf(ctx context.Context) FleetCtx {
	if f := FleetCtxFrom(ctx); !f.Empty() {
		return f
	}
	return FleetCtxFromEnv()
}

// fleetMetaOf renders the _meta fragment for f. Nil when absent, so callers
// leave _meta untouched rather than stamping an empty marker.
func fleetMetaOf(f FleetCtx) map[string]json.RawMessage {
	if f.Empty() {
		return nil
	}
	out := map[string]json.RawMessage{}
	if f.TaskID != "" {
		if b, err := json.Marshal(f.TaskID); err == nil {
			out[FleetMetaTaskID] = b
		}
	}
	if f.TaskName != "" {
		if b, err := json.Marshal(f.TaskName); err == nil {
			out[FleetMetaTaskName] = b
		}
	}
	return out
}

// FleetHeadersOf renders the HTTP header carriers for f. Nil when absent.
func FleetHeadersOf(f FleetCtx) map[string]string {
	if f.Empty() {
		return nil
	}
	out := map[string]string{}
	if f.TaskID != "" {
		out[FleetHeaderTaskID] = f.TaskID
	}
	if f.TaskName != "" {
		out[FleetHeaderTaskName] = f.TaskName
	}
	return out
}

// withFleetMetaParams merges the effective fleet _meta into params,
// set-if-absent so a caller's deliberate value is never overwritten.
// It covers the legacy-era path, where callReissue sends params without
// withMeta (no per-request version exists to carry); on the modern path
// withMeta repeats the same merge idempotently. Absent context stamps
// nothing: params pass through untouched.
func withFleetMetaParams(params json.RawMessage, ctx context.Context) json.RawMessage {
	frag := fleetMetaOf(fleetCtxOf(ctx))
	if len(frag) == 0 {
		return params
	}
	m := map[string]json.RawMessage{}
	if err := json.Unmarshal(params, &m); err != nil {
		return params
	}
	meta := map[string]json.RawMessage{}
	if raw, ok := m["_meta"]; ok {
		_ = json.Unmarshal(raw, &meta)
	}
	changed := false
	for k, v := range frag {
		if _, ok := meta[k]; !ok {
			meta[k] = v
			changed = true
		}
	}
	if !changed {
		return params
	}
	b, err := json.Marshal(meta)
	if err != nil {
		return params
	}
	m["_meta"] = b
	out, err := json.Marshal(m)
	if err != nil {
		return params
	}
	return out
}

// withFleetHeaders merges the effective fleet headers into ctx's extra
// headers, preserving whatever is already there (e.g. Mcp-Param-*).
func withFleetHeaders(ctx context.Context) context.Context {
	h := FleetHeadersOf(fleetCtxOf(ctx))
	if len(h) == 0 {
		return ctx
	}
	merged := map[string]string{}
	if prev, ok := ctx.Value(extraHeadersKey{}).(map[string]string); ok {
		for k, v := range prev {
			merged[k] = v
		}
	}
	for k, v := range h {
		// An explicit header already on the request wins: the caller's
		// deliberate value is never overwritten by an ambient env.
		if _, ok := merged[k]; !ok {
			merged[k] = v
		}
	}
	return context.WithValue(ctx, extraHeadersKey{}, merged)
}
