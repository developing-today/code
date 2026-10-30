package daemon

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"time"

	"github.com/dezren39/mcpx/internal/defaults"
	"github.com/dezren39/mcpx/internal/elicit"
	"github.com/dezren39/mcpx/internal/events"
	"github.com/dezren39/mcpx/internal/mcpclient"
	"github.com/dezren39/mcpx/internal/pool"
)

// InstallHooks connects every pool to the event bus and the question broker.
//
// Called once the registry and the bus both exist. Done as a separate step
// rather than inside NewRegistry because the bus belongs to the server, and a
// registry built for a test should not need one.
func (r *Registry) InstallHooks(bus *events.Bus, broker *elicit.Broker, roots []mcpclient.Root) {
	r.Events = bus
	r.broker = broker
	hooks := &pool.Hooks{
		Roots:    roots,
		LogLevel: "info",

		OnMessage: func(server string, m mcpclient.ServerMessage) {
			r.publish(events.Event{Kind: events.ServerLog, Server: server, Data: mustJSON(m)})
		},
		OnProgress: func(server string, p mcpclient.Progress) {
			r.publish(events.Event{Kind: events.Progress, Server: server, Data: mustJSON(p)})
		},
		OnListChanged: func(server, kind string) {
			// A list change means the cached schema is stale. Dropping it
			// here is what makes the next catalog reflect reality rather than
			// whatever was true when the daemon started.
			// Through Pool, which holds r.mu: this runs on an upstream
			// client's goroutine, any time, and Reload replaces r.pools.
			if p, ok := r.Pool(server); ok {
				p.Invalidate()
			}
			k := map[string]events.Kind{
				"tools": events.ToolsChanged, "resources": events.ResourcesChanged,
				"prompts": events.PromptsChanged,
			}[kind]
			if k != "" {
				r.publish(events.Event{Kind: k, Server: server})
			}
			// A changed catalog can orphan an argument rule (#1986):
			// re-check this server's rules against what is now cached.
			if kind == "tools" {
				r.warnDeadArgumentRulesFor(server)
			}
		},
		OnResourceUpdated: func(server, uri string) {
			r.publish(events.Event{Kind: events.ResourceUpdated, Server: server, URI: uri})
		},
		OnElicitationComplete: func(server, id string) {
			// The server says the out-of-band flow finished. The question it
			// was attached to is therefore answered, and whatever is waiting
			// on it should hear so now rather than at its deadline.
			if r.broker != nil {
				_ = r.broker.Respond(elicit.Answer{
					ID: brokerRowID(server, id), Action: elicit.Accept, By: "server:" + server,
				})
			}
			r.publish(events.Event{
				Kind: events.ElicitCompleted, Server: server,
				Data: mustJSON(map[string]string{"elicitationId": id}),
			})
		},
		Elicit: r.answerServer,
	}
	if broker != nil {
		hooks.URLElicitation = r.urlElicitationsViaBroker
	}
	r.upstreamHooks(hooks)
	// Under r.mu: Reload reads r.hooks and replaces r.pools under it.
	r.mu.Lock()
	defer r.mu.Unlock()
	r.hooks = hooks
	seen := map[*pool.Pool]bool{}
	for _, p := range r.pools {
		if seen[p] {
			continue
		}
		seen[p] = true
		p.Hooks = hooks
	}
}

func (r *Registry) publish(e events.Event) {
	if r.Events != nil {
		r.Events.Publish(e)
	}
}

// elicitEventSummary is the only shape of an elicitation that may go
// on the event bus (#1677, design #1675 Phase 2): id, server, tool,
// mode, audience, state, expiry. Never a question's message, schema or
// url, and never an answer's content -- the bus has no field-level
// filtering, so every future subscriber would inherit whatever is
// published here. Anyone who needs the question itself reads it from
// the broker (Broker.Get / Broker.Await), which is the read path the
// shrink points consumers at.
func elicitEventSummary(req elicit.Request) map[string]any {
	return map[string]any{
		"id":       req.ID,
		"server":   req.Server,
		"tool":     req.Tool,
		"mode":     string(req.Mode),
		"audience": string(req.Audience),
		"state":    string(req.State),
		"expiry":   req.ExpiresAt,
	}
}

// elicitAnsweredSummary is elicitEventSummary for the answer event:
// the question's identity fields, with the state the answer settled
// it into (expired when the answer came from expiry, per Respond).
func elicitAnsweredSummary(req elicit.Request, ans elicit.Answer) map[string]any {
	out := elicitEventSummary(req)
	out["state"] = string(elicit.Answered)
	if ans.By == "expiry" {
		out["state"] = string(elicit.Expired)
	}
	return out
}

func mustJSON(v any) json.RawMessage {
	b, err := json.Marshal(v)
	if err != nil {
		return nil
	}
	return b
}

// answerServer handles a request a server sent to mcpx.
//
// Two methods land here, and they are the same shape seen from different
// angles. Elicitation asks a *person or agent* a question. Sampling asks a
// *model* to write something. mcpx is neither a person nor a model, so both
// become a stored question that whatever drives mcpx can answer -- which is
// exactly what "passthrough" means.
func (r *Registry) answerServer(ctx context.Context, server, key, method string, params json.RawMessage) (any, error) {
	if r.broker == nil {
		// Nothing can answer, so say so rather than hang. Cancel is honest:
		// nobody was asked, so nobody chose.
		if method == "elicitation/create" {
			return map[string]any{"action": "cancel"}, nil
		}
		return nil, fmt.Errorf("mcpx cannot answer %s: no broker", method)
	}

	switch method {
	case "elicitation/create":
		return r.elicitViaBroker(ctx, server, key, params)
	case "sampling/createMessage":
		return r.sampleViaBroker(ctx, server, key, params)
	case "roots/list":
		return r.rootsViaBroker(ctx, server, key, params)
	}
	return nil, fmt.Errorf("mcpx does not implement %s", method)
}

// brokerRowID is the broker row key for a server-supplied elicitation
// ID: the (server, elicitationId) pair, length-prefixed so the encoding
// is unambiguous. Plain concatenation would collide whenever a server
// name or an elicitation ID contains ":" -- server "a" with ID "b:c"
// and server "a:b" with ID "c" would share one row, and a completion
// from one could settle the other's question. An empty elicitationId
// (a minted ID, already unique) stays bare.
func brokerRowID(server, elicitationID string) string {
	if elicitationID == "" {
		return elicitationID
	}
	return strconv.Itoa(len(server)) + ":" + server + ":" + elicitationID
}

func (r *Registry) elicitViaBroker(ctx context.Context, server, key string, params json.RawMessage) (any, error) {
	var p struct {
		Mode            string          `json:"mode"`
		Message         string          `json:"message"`
		RequestedSchema json.RawMessage `json:"requestedSchema"`
		URL             string          `json:"url"`
		ElicitationID   string          `json:"elicitationId"`
	}
	if err := json.Unmarshal(params, &p); err != nil {
		return nil, err
	}
	mode := elicit.Form
	if p.Mode == "url" {
		mode = elicit.URL
	}
	// Broker rows are keyed by ID alone, and elicitationId is the
	// upstream's own choice: two servers can pick the same one and the
	// second OpenRequest would fail on the primary key (ELI-09). A
	// server-supplied ID is therefore stored under brokerRowID;
	// OnElicitationComplete composes the same row ID before it
	// responds. Minted IDs (empty elicitationId) are already unique and
	// stay bare. Rows opened under a bare ID before this fix are not
	// migrated; they expire by TTL.
	brokerID := brokerRowID(server, p.ElicitationID)
	req, err := r.broker.OpenRequest(elicit.Request{
		ID:        brokerID,
		Server:    server,
		Mode:      mode,
		Message:   p.Message,
		Schema:    p.RequestedSchema,
		URL:       p.URL,
		Trace:     r.askIDFor(ctx, server, key),
		ExpiresAt: time.Now().Add(defaults.ElicitTTL),
	})
	if err != nil {
		return nil, err
	}
	// Bound to the call it interrupted, where one can be identified, so a
	// client of mcpx can be handed it inline. Where one cannot -- two
	// callers sharing an instance -- it stays a broker question and its
	// audience answers, which is the behaviour that existed before any of
	// this and is still correct.
	callID, _ := r.attach(ctx, server, key, req, "elicitation/create", params)
	r.publish(events.Event{Kind: events.ElicitOpened, Server: server, Trace: callID,
		Data: mustJSON(elicitEventSummary(req))})

	ans, err := r.broker.Await(ctx, req.ID)
	if err != nil {
		return map[string]any{"action": "cancel"}, nil
	}
	r.publish(events.Event{Kind: events.ElicitAnswered, Server: server, Data: mustJSON(elicitAnsweredSummary(req, ans))})

	out := map[string]any{"action": string(ans.Action)}
	if ans.Action == elicit.Accept && len(ans.Content) > 0 {
		out["content"] = json.RawMessage(ans.Content)
	}
	return out, nil
}

// urlElicitationsViaBroker opens the URL elicitations an upstream's
// -32042 tools/call error named (#2079, Phase 2 of #2077). Each entry
// runs the existing elicitation/create path with synthesised url-mode
// params -- no second URL flow -- sequentially, because elicitViaBroker
// blocks on the broker answer. Every entry must be accepted before the
// client retries the call; the first decline, cancel, or expiry stops
// here with a named error and the remaining entries are never opened.
func (r *Registry) urlElicitationsViaBroker(ctx context.Context, server, key string, elicitations []mcpclient.URLElicitation) (bool, error) {
	if r.broker == nil {
		return false, fmt.Errorf("mcpx cannot open URL elicitations from %s: no broker", server)
	}
	for _, e := range elicitations {
		params, err := json.Marshal(map[string]any{
			"mode": "url", "url": e.URL, "elicitationId": e.ElicitationID, "message": e.Message,
		})
		if err != nil {
			return false, err
		}
		out, err := r.elicitViaBroker(ctx, server, key, params)
		if err != nil {
			return false, err
		}
		res, _ := out.(map[string]any)
		action, _ := res["action"].(string)
		if action != string(elicit.Accept) {
			return false, fmt.Errorf("the URL elicitation %s from %s was %s", e.ElicitationID, server, action)
		}
	}
	return true, nil
}

// sampleViaBroker turns a sampling request into a question for whatever is
// driving mcpx.
//
// The request carries messages, a system prompt, a token ceiling and model
// preferences. None of that is interpreted here: mcpx has no model to give
// it to. It is stored exactly as sent, and whoever answers -- the opencode
// plugin handing it to the session's own model, a person, a script --
// produces the completion.
func (r *Registry) sampleViaBroker(ctx context.Context, server, key string, params json.RawMessage) (any, error) {
	var p struct {
		MaxTokens    int    `json:"maxTokens"`
		SystemPrompt string `json:"systemPrompt"`
	}
	_ = json.Unmarshal(params, &p)

	// The request is checked against the era the upstream negotiated before
	// anybody is asked: a message list that is not sampling's shape is the
	// server's mistake, and -32602 says so without a question nobody could
	// answer well. No era on the context means this was called outside a
	// connection's answer path (tests), where the permissive union applies.
	era, known := mcpclient.SamplingEraFrom(ctx)
	arrayOK := !known || era.Negotiated == "" || era.Negotiated >= "2025-11-25"
	if err := elicit.ValidateSampleRequest(params, arrayOK); err != nil {
		return nil, &elicit.SamplingInvalidParams{Err: err}
	}

	req, err := r.broker.OpenRequest(elicit.Request{
		Server: server,
		Trace:  r.askIDFor(ctx, server, key),
		Mode:   elicit.Sample,
		// A sampling request has no question as such. The system prompt is
		// the closest thing to one, and it is what a human reading the
		// queue needs to see first.
		Message:   firstNonEmptyStr(p.SystemPrompt, server+" asks for a model completion"),
		Schema:    params,
		Audience:  elicit.ToAgent,
		Reason:    "sampling asks a model, and the agent driving mcpx has one",
		ExpiresAt: time.Now().Add(defaults.ElicitTTL),
	})
	if err != nil {
		return nil, err
	}
	callID, _ := r.attach(ctx, server, key, req, "sampling/createMessage", params)
	r.publish(events.Event{Kind: events.SampleOpened, Server: server, Trace: callID,
		Data: mustJSON(elicitEventSummary(req))})

	ans, err := r.broker.Await(ctx, req.ID)
	if err != nil {
		return nil, err
	}
	r.publish(events.Event{Kind: events.SampleAnswered, Server: server, Data: mustJSON(elicitAnsweredSummary(req, ans))})

	// An expired wait is not a refusal: nobody was there to refuse. The
	// broker synthesizes the expiry answer itself (Action cancel, By
	// "expiry"), so By is the only thing that tells it apart from a real
	// cancel -- and the upstream must hear it as an internal failure
	// (-32603) that names the expiry, never as a user rejection.
	if ans.By == "expiry" {
		return nil, mcpclient.InternalError("the sampling request expired before anyone answered it")
	}
	// Declining a sampling request is a refusal the server must hear as an
	// error; there is no "decline" result shape for it in the specification.
	// The 2025-11-25 sampling error table assigns it -1, so the upstream
	// can tell a refusal apart from an internal failure.
	if ans.Action != elicit.Accept {
		return nil, mcpclient.UserRejectedError("the sampling request was " + string(ans.Action))
	}
	var result map[string]any
	if err := json.Unmarshal(ans.Content, &result); err != nil {
		return nil, fmt.Errorf("a sampling answer must be a CreateMessageResult: %w", err)
	}
	// JSON null unmarshals into a nil map without error, and the writes
	// below then panicked -- on an upstream client's goroutine, where nothing
	// recovers, so one POSTed answer took the daemon down.
	if result == nil {
		return nil, errors.New("a sampling answer must be a CreateMessageResult, not null")
	}
	// Role and model are no longer filled in when the answerer leaves them
	// out: a result that is not a CreateMessageResult is refused upstream as
	// -32602, and inventing a role or a model for it hid that from the
	// server that asked. A model hint is still not checked -- hints are
	// expressly ignorable, so a mismatch is not a failure.
	var carriedTools struct {
		Tools json.RawMessage `json:"tools"`
	}
	_ = json.Unmarshal(params, &carriedTools)
	tools := len(carriedTools.Tools) > 0 && string(carriedTools.Tools) != "null"
	if err := elicit.ValidateSampleResult(ans.Content, arrayOK, tools); err != nil {
		return nil, &elicit.SamplingInvalidParams{Err: err}
	}
	return result, nil
}

// rootsViaBroker relays a server's roots/list to the client whose call it
// interrupted. Only to that client: a roots question nobody's call raised has
// no one to answer it but mcpx, which answers with its own configured roots
// (mcpclient.ErrNotRelayed).
func (r *Registry) rootsViaBroker(ctx context.Context, server, key string, params json.RawMessage) (any, error) {
	if r.askIDFor(ctx, server, key) == "" {
		return nil, mcpclient.ErrNotRelayed
	}
	req, err := r.broker.OpenRequest(elicit.Request{
		Server:    server,
		Trace:     r.askIDFor(ctx, server, key),
		Mode:      elicit.Roots,
		Message:   server + " asks for the client's roots",
		Audience:  elicit.ToAgent,
		Reason:    "roots are the client's own, and only it can list them",
		ExpiresAt: time.Now().Add(defaults.ElicitTTL),
	})
	if err != nil {
		return nil, err
	}
	r.attach(ctx, server, key, req, "roots/list", params)
	ans, err := r.broker.Await(ctx, req.ID)
	if err != nil {
		return nil, err
	}
	if ans.Action != elicit.Accept {
		return nil, errors.New("the roots request was " + string(ans.Action))
	}
	var result map[string]any
	if err := json.Unmarshal(ans.Content, &result); err != nil {
		return nil, fmt.Errorf("a roots answer must be a ListRootsResult: %w", err)
	}
	// null is not a ListRootsResult: relayed, it reached the server as one.
	if result == nil {
		return nil, errors.New("a roots answer must be a ListRootsResult, not null")
	}
	return result, nil
}

// OpenBroker opens the question store. The store lives in its own
// owner-only directory under the state dir (elicit.StorePath); a store
// left beside the log index by an older build is migrated on open.
func OpenBroker(paths Paths) (*elicit.Broker, error) {
	dir := filepath.Join(paths.State, "logs")
	if err := os.MkdirAll(dir, defaults.PublicDirMode); err != nil {
		return nil, err
	}
	return elicit.OpenAt(paths.State, filepath.Join(dir, "elicit.db"))
}

// PublishLifecycle forwards a pool lifecycle event onto the bus.
//
// The pool reports server starts, stops and tool calls through one callback,
// named by dotted strings. They map onto the bus's own kinds where one exists;
// anything else is passed through under its own name, so a subscriber asking
// for "server" hears every server event without this function having to
// enumerate them.
func (s *Server) PublishLifecycle(event string, attrs map[string]any) {
	if s.Events == nil {
		return
	}
	kind := events.Kind(event)
	switch event {
	case "server.start":
		kind = events.ServerStarted
	case "server.stop":
		kind = events.ServerStopped
	case "mcp.call":
		kind = events.CallFinished
	}
	e := events.Event{Kind: kind, Data: mustJSON(attrs)}
	if v, ok := attrs["server"].(string); ok {
		e.Server = v
	}
	if v, ok := attrs["session"].(string); ok {
		e.Session = v
	}
	if v, ok := attrs["trace"].(string); ok {
		e.Trace = v
	}
	s.Events.Publish(e)
}

// askIDFor names the interruptible call a question belongs to, or "" when
// none can be identified.
func (r *Registry) askIDFor(ctx context.Context, server, key string) string {
	if r.asks == nil {
		return ""
	}
	if a, ok := r.askFor(ctx, server, key); ok {
		return a.ID
	}
	return ""
}
