package cli

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"os"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/dezren39/mcpx/internal/daemon"
	"github.com/dezren39/mcpx/internal/defaults"
	"github.com/dezren39/mcpx/internal/execsvc"
	"github.com/dezren39/mcpx/internal/mcpclient"
	"github.com/dezren39/mcpx/internal/mcpserver"
)

// daemonAsker runs a request the upstream server may interrupt.
//
// It is the bridge between the two halves of the question: mcpx's MCP server
// knows a client that can answer one, and the daemon knows the call that
// raised it. Neither can see the other, so this sits between them and speaks
// /v1 -- the same /v1 a curl user would, which is what keeps the two
// surfaces honest about each other.
type daemonAsker struct{ app *App }

func (d daemonAsker) client(ctx context.Context) (*Client, error) {
	return d.app.ensure(ctx)
}

// Begin translates one of mcpx's own tools into the upstream call behind it.
//
// Only `mcpx_call` and the two pass-through methods reach a server that
// could ask anything. Everything else -- a catalogue read, a script, a /v1
// operation -- answers ErrNotInterruptible and is run the ordinary way.
func (d daemonAsker) Begin(ctx context.Context, kind string, params json.RawMessage) (string, error) {
	body := map[string]any{"kind": kind, "context": d.app.mcpCaller(ctx)}
	ctx = mcpclient.WithCallerVersion(ctx, mcpserver.PeerVersion(ctx))

	switch kind {
	case "tools/call":
		// Decode only the name and the opaque arguments first. The
		// gateway-specific argument fields (mcpx_call's namespace /
		// tool strings) are decoded in the gateway branch below, and
		// only there: a raw or pass-through tool's own arguments are
		// the upstream's, and an upstream argument that happens to be
		// named namespace or tool with a non-string value must not
		// fail this decode and cost the call its ask attribution.
		var p struct {
			Name      string          `json:"name"`
			Arguments json.RawMessage `json:"arguments"`
		}
		if err := json.Unmarshal(params, &p); err != nil {
			return "", mcpserver.ErrNotInterruptible
		}
		// A raw tool (codemode=false) is an upstream call under a
		// namespaced name -- <namespace>_<tool> -- and is just as likely
		// to ask as a pass-through one. The request's own raw lookup
		// resolves it to the namespace and the upstream's own name;
		// without this the name fell to ErrNotInterruptible below and the
		// call ran with no ask attribution. Known-and-unclaimed means
		// the request asked for the raw surface and this name is not on
		// it: not interruptible here either, and the ordinary path
		// answers for the name. Checked before pass-through, as invoke
		// checks the raw surface first on a raw request.
		if ns, tool, known := mcpserver.RawToolOf(ctx, p.Name); known {
			if ns == "" {
				return "", mcpserver.ErrNotInterruptible
			}
			body["server"], body["tool"] = ns, tool
			if len(p.Arguments) > 0 {
				body["args"] = p.Arguments
			}
			// The upstream judges what the client can answer, as on the
			// direct path (mcpBackend.Call).
			ctx = mcpclient.WithClientCapabilities(ctx, mcpserver.DeclaredCapabilities(ctx))
			break
		}
		// A pass-through tool is an upstream call under its own name, and
		// the one most likely to ask: it was left to the broker, where no
		// client could answer and the request hung. Checked before the
		// gateway tools, because on a collision the upstream's tool is
		// the one the name means.
		if ns := d.passTool(ctx, p.Name); ns != "" {
			body["server"], body["tool"] = ns, p.Name
			if len(p.Arguments) > 0 {
				body["args"] = p.Arguments
			}
			// The upstream judges what the client can answer, as on the
			// direct path (mcpBackend.Call).
			ctx = mcpclient.WithClientCapabilities(ctx, mcpserver.DeclaredCapabilities(ctx))
			break
		}
		if p.Name == "mcpx_exec" {
			// A script is one call making many; the daemon correlates its
			// questions through the run id rather than a single (server, key).
			var e struct {
				Source     string `json:"source"`
				TimeoutSec int    `json:"timeoutSec"`
			}
			if json.Unmarshal(p.Arguments, &e) != nil || e.Source == "" {
				return "", mcpserver.ErrNotInterruptible
			}
			c, err := d.client(ctx)
			if err != nil {
				return "", err
			}
			// The same preparation the direct path does before /v1/exec.
			if err := d.app.ensureAnySchemas(ctx, c); err != nil {
				return "", err
			}
			body["kind"] = "exec"
			body["source"] = e.Source
			body["options"] = mcpBackend{app: d.app}.execOptions(ctx, e.TimeoutSec)
			break
		}
		if p.Name != "mcpx_call" {
			return "", mcpserver.ErrNotInterruptible
		}
		// The gateway branch: only now are mcpx_call's own argument
		// fields decoded, so only a malformed mcpx_call -- not a raw
		// tool with upstream-shaped arguments -- fails on them.
		var g struct {
			Namespace string          `json:"namespace"`
			Tool      string          `json:"tool"`
			Arguments json.RawMessage `json:"arguments"`
		}
		if err := json.Unmarshal(p.Arguments, &g); err != nil {
			return "", mcpserver.ErrNotInterruptible
		}
		ns, tool := g.Namespace, g.Tool
		if tool == "" {
			// A dotted name in the namespace field is what somebody sends,
			// because that is how the tools are written everywhere else.
			if a, b, ok := strings.Cut(ns, "."); ok {
				ns, tool = a, b
			}
		}
		if ns == "" || tool == "" {
			return "", mcpserver.ErrNotInterruptible
		}
		body["server"], body["tool"] = ns, tool
		if len(g.Arguments) > 0 {
			body["args"] = g.Arguments
		}

	case "prompts/get":
		var p struct {
			Name      string            `json:"name"`
			Arguments map[string]string `json:"arguments"`
		}
		if err := json.Unmarshal(params, &p); err != nil {
			return "", mcpserver.ErrNotInterruptible
		}
		c, err := d.client(ctx)
		if err != nil {
			return "", err
		}
		server, name, err := d.app.resolvePrompt(ctx, c, p.Name)
		if err != nil {
			return "", err
		}
		body["server"], body["name"], body["arguments"] = server, name, p.Arguments

	case "resources/read":
		var p struct {
			URI string `json:"uri"`
		}
		if err := json.Unmarshal(params, &p); err != nil {
			return "", mcpserver.ErrNotInterruptible
		}
		ns, rest, ok := d.app.resolveURI(ctx, p.URI)
		if !ok {
			return "", mcpserver.ErrNotInterruptible
		}
		body["server"], body["uri"] = ns, rest

	default:
		return "", mcpserver.ErrNotInterruptible
	}

	c, err := d.client(ctx)
	if err != nil {
		return "", err
	}
	// Outside the profile, the direct path answers -- with the same refusal
	// it gives every client, rather than the ask path starting the call.
	// Only a request that names a server has one to check: a script
	// (kind "exec") names none, and treating "" as a hidden namespace sent
	// every mcpx_exec down the direct path, where its questions could not
	// reach the client.
	if server, named := body["server"].(string); named {
		visible, err := d.app.visibleNamespaces(ctx, c)
		if err != nil {
			return "", err
		}
		if !visible(server) {
			return "", mcpserver.ErrNotInterruptible
		}
	}
	if r := mcpserver.RelayFrom(ctx); r != nil && kind == "tools/call" {
		body["relay"] = daemon.CallRelay{ProgressToken: r.ProgressToken, LogLevel: r.LogLevel, Meta: r.Meta}
	}
	raw, err := c.do(ctx, http.MethodPost, "/v1/ask", body)
	if err != nil {
		return "", err
	}
	var out struct {
		CallID string `json:"callId"`
	}
	if err := json.Unmarshal(raw, &out); err != nil {
		return "", err
	}
	if out.CallID == "" {
		return "", fmt.Errorf("the daemon started a call but named no id")
	}
	return out.CallID, nil
}

// codeMissingCapability is 2026-07-28's MissingRequiredClientCapability.
const codeMissingCapability = -32021

type askPollReply struct {
	Status    string                     `json:"status"`
	Done      bool                       `json:"done"`
	Error     string                     `json:"error"`
	Questions []mcpserver.Question       `json:"questions"`
	Result    map[string]json.RawMessage `json:"result"`
	// Notifications are what the call relayed since the last poll.
	Notifications []mcpserver.Notification `json:"notifications"`
	Upstream      *daemon.UpstreamError    `json:"upstream"`
}

func (d daemonAsker) Poll(ctx context.Context, callID string, wait time.Duration) (mcpserver.Outcome, error) {
	c, err := d.client(ctx)
	if err != nil {
		return mcpserver.Outcome{}, err
	}
	ms := int(wait / time.Millisecond)
	if ms < 1 {
		ms = 1
	}
	raw, err := c.do(ctx, http.MethodGet,
		"/v1/ask/"+callID+"?waitMs="+strconv.Itoa(ms), nil)
	if err != nil {
		return mcpserver.Outcome{}, err
	}
	var reply askPollReply
	if err := json.Unmarshal(raw, &reply); err != nil {
		return mcpserver.Outcome{}, err
	}
	out := mcpserver.Outcome{Done: reply.Done, Questions: reply.Questions, Notifications: reply.Notifications}
	if !reply.Done {
		return out, nil
	}
	if reply.Error != "" {
		// A failed upstream call is a tool result with isError, not a
		// protocol error: a client that retries the wrong thing on a tool
		// failure never converges.
		out.Text, out.IsError = reply.Error, true
		// The protocol layer relays this as the error itself when the
		// tool is a pass-through upstream's, and ignores it otherwise.
		out.Upstream = upstreamError(errors.New(reply.Error))
		// Classified for the two methods whose failure is a protocol
		// error; which one applies is the caller's to pick, since the
		// same upstream -32602 means not-found for a read and a bad
		// argument for a prompt.
		failed := errors.New(reply.Error)
		switch {
		case reply.Upstream != nil && reply.Upstream.Code == codeMissingCapability:
			// Only -32021 here: the ask path also runs mcpx_call, whose
			// upstream errors are tool results, and Poll cannot tell which.
			out.Err = &mcpserver.UpstreamError{Code: reply.Upstream.Code, Message: reply.Upstream.Message, Data: reply.Upstream.Data}
		case upstreamNotFound(failed) && upstreamInvalid(failed):
			out.Err = fmt.Errorf("%w, %w: %v", mcpserver.ErrResourceNotFound, mcpserver.ErrInvalidParams, failed)
		case upstreamNotFound(failed):
			out.Err = fmt.Errorf("%w: %v", mcpserver.ErrResourceNotFound, failed)
		}
		return out, nil
	}
	if string(reply.Result["kind"]) == `"exec"` {
		var res execsvc.Result
		if err := json.Unmarshal(reply.Result["result"], &res); err != nil {
			return out, err
		}
		text, rerr := renderExec(res)
		if rerr != nil {
			out.Text, out.IsError = rerr.Error(), true
			return out, nil
		}
		out.Text = text
		return out, nil
	}
	out.Text, out.Contents, out.IsError = d.renderAsk(reply.Result)
	return out, nil
}

// renderAsk turns the daemon's task result into what every other mcpx
// result is, using the same renderers the direct path uses. Two ways to
// render one result is two ways for them to disagree.
func (d daemonAsker) renderAsk(result map[string]json.RawMessage) (text string, contents []mcpserver.ResourceContents, failed bool) {
	var kind, server string
	_ = json.Unmarshal(result["kind"], &kind)
	_ = json.Unmarshal(result["server"], &server)
	inner := result["result"]
	switch kind {
	case "resources/read":
		return "", d.app.resourceContents(inner, server), false
	default:
		// tools/call and prompts/get: verbatim, as the direct path sends
		// them (#206).
		return mcpserver.EncodeRaw(d.app.exposeResult(server, inner)), nil, false
	}
}

func (d daemonAsker) Reply(ctx context.Context, callID string, answers map[string]json.RawMessage) error {
	c, err := d.client(ctx)
	if err != nil {
		return err
	}
	_, err = c.do(ctx, http.MethodPost, "/v1/ask/"+callID+"/answers",
		map[string]any{"answers": answers})
	return err
}

func (d daemonAsker) Abandon(callID string) {
	// Best effort, on its own deadline: the request that wanted this is
	// already being failed, and its context is cancelled or about to be.
	ctx, cancel := context.WithTimeout(context.Background(), defaults.ProtoAbandonGrace)
	defer cancel()
	if c, err := d.client(ctx); err == nil {
		_, _ = c.do(ctx, http.MethodPost, "/v1/ask/"+callID+"/abandon", nil)
	}
}

// lazyMCP builds mcpx's MCP server the first time something asks for it,
// and rebuilds it when its declared inputs change (PM-13, #1942).
//
// Built lazily because constructing it reads adapter declarations and may
// fetch a declared OpenAPI document over the network. The daemon auto-starts
// on the first command anybody runs, and a remote document that is slow to
// answer would become a daemon that is slow to start -- for a surface that
// particular invocation is not using.
//
// The old build used a sync.Once: the surface could never change for the
// daemon's life (adapters added later were invisible) and a failed first
// build -- even one caused by the first request's cancelled context -- was
// cached forever. Now the build is keyed by a fingerprint of the adapter
// and API declaration files: unchanged inputs reuse the built surface, a
// change rebuilds it (live MCP sessions on the old surface end with it,
// which is still strictly better than the restart this used to require),
// and a failure is returned without being cached, so the next request
// retries.
type lazyMCP struct {
	app *App
	// build and inputs are seams for tests; nil means the App's own.
	build  func(ctx context.Context) (*mcpserver.Server, error)
	inputs func() string
	retire func(*mcpserver.Server)

	mu      sync.Mutex
	srv     *mcpserver.Server
	builtFP string
}

// fingerprint hashes the declared file inputs the surface is built
// from -- the adapter files and the API declaration files -- so a
// changed, added, or newly-created declaration changes the value and
// the surface is rebuilt. A file that cannot be read contributes its
// error to the hash: present-but-broken and absent are different
// states, and a file appearing later must not match a fingerprint
// taken while it was missing.
func (l *lazyMCP) fingerprint() string {
	if l.inputs != nil {
		return l.inputs()
	}
	h := sha256.New()
	for _, key := range []string{"paths.adapters", "paths.apis"} {
		for _, p := range splitPathList(l.app.Settings().String(key)) {
			b, err := os.ReadFile(p)
			fmt.Fprintf(h, "%s\x00", p)
			if err != nil {
				fmt.Fprintf(h, "err:%v\x00", err)
				continue
			}
			h.Write(b)
			h.Write([]byte("\x00"))
		}
	}
	return hex.EncodeToString(h.Sum(nil))
}

func (l *lazyMCP) get(ctx context.Context) (*mcpserver.Server, error) {
	fp := l.fingerprint()
	l.mu.Lock()
	defer l.mu.Unlock()
	if l.srv != nil && fp == l.builtFP {
		return l.srv, nil
	}
	build := l.build
	if build == nil {
		build = l.app.MCPServer
	}
	srv, err := build(ctx)
	if err != nil {
		// PM-13 (#1942): a failed build is NOT cached. The old
		// sync.Once stored the first error for the daemon's life --
		// including a first request whose context was cancelled --
		// so one transient failure took /mcp down until a restart.
		// The fingerprint stays unbuilt, and the next request
		// retries. (A previously built surface is likewise not
		// served stale past a known input change: the caller gets
		// the error, not a surface that no longer matches config.)
		return nil, err
	}
	old := l.srv
	l.srv, l.builtFP = srv, fp
	if old != nil && old != srv {
		// PM-13 follow-up (#1942): the replaced surface is retired, not
		// abandoned. Its legacy sessions would otherwise keep their GET
		// streams, notification subscriptions, and listen streams open
		// against a server nothing routes to, accumulating with every
		// rebuild. Clients re-initialize against the new surface.
		retire := l.retire
		if retire == nil {
			retire = (*mcpserver.Server).CloseSessions
		}
		retire(old)
	}
	return srv, nil
}

func (l *lazyMCP) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	srv, err := l.get(r.Context())
	if err != nil {
		http.Error(w, "mcpx could not build its MCP surface: "+err.Error(),
			http.StatusInternalServerError)
		return
	}
	srv.ServeHTTP(w, r)
}

// InvokeTool runs one tool, for the plain-POST projection of the surface.
func (l *lazyMCP) InvokeTool(ctx context.Context, tool string, args json.RawMessage) (string, error) {
	srv, err := l.get(ctx)
	if err != nil {
		return "", err
	}
	return srv.InvokeTool(ctx, tool, args)
}

// passTool is the pass-through upstream serving name, or "".
func (d daemonAsker) passTool(ctx context.Context, name string) string {
	// The protocol layer has normally looked this up already for the request.
	if ns, known := mcpserver.PassToolOf(ctx, name); known {
		return ns
	}
	for _, ns := range d.app.passNS() {
		tools, err := mcpBackend{app: d.app}.UpstreamTools(ctx, ns)
		if err != nil {
			continue
		}
		for _, t := range tools {
			if t.Name == name {
				return ns
			}
		}
	}
	return ""
}
