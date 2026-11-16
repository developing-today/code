// Package client is the public Go client for the mcpx daemon's /v1 API.
//
// Every call this package makes is derived at run time from the daemon's
// own operation table, internal/api.Ops: the method, the path, and where
// each parameter travels (path, query, or body) are read from the table,
// never hand-written per operation. A new /v1 operation therefore appears
// in this client the moment it is declared in the table, and the client
// cannot drift from the daemon the way a hand-written one would — the
// same single-source property the CLI, the MCP tools, and the OpenAPI
// document already get from the table.
//
// Stability promise: this package is pre-1.0 — public but unstable.
// Within the pre-1.0 period the exported surface may change in any
// release: signatures may be renamed, reshaped, or removed without a
// deprecation period, and callers must expect to adjust on upgrade.
// What will not change silently is the derivation rule itself: as long
// as an operation is in the table, Call reaches it by its table name.
package client

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"reflect"
	"strings"

	"github.com/developing-today/code/pkgs/mcpx/internal/api"
)

// Client talks to one mcpx daemon over HTTP at its /v1 base endpoint,
// for example http://127.0.0.1:7432 (the daemon's loopback endpoint, as
// reported by the health operation).
type Client struct {
	base  string
	token string
	hc    *http.Client
}

// Option configures a Client.
type Option func(*Client)

// WithToken sets the bearer token sent on every request, matching the
// daemon's Authorization: Bearer convention. An empty token sends no
// header, which is the loopback daemon's usual case.
func WithToken(token string) Option {
	return func(c *Client) { c.token = token }
}

// WithHTTPClient replaces the HTTP client, for callers that need a
// custom transport (a unix-socket dialler, a test round-tripper, TLS).
func WithHTTPClient(hc *http.Client) Option {
	return func(c *Client) {
		if hc != nil {
			c.hc = hc
		}
	}
}

// New builds a Client for the daemon at baseURL. The trailing slash is
// optional; operation paths from the table are appended to it.
func New(baseURL string, opts ...Option) *Client {
	c := &Client{base: strings.TrimRight(baseURL, "/"), hc: http.DefaultClient}
	for _, opt := range opts {
		opt(c)
	}
	return c
}

// Operations returns the daemon's /v1 operation table: every operation
// Call can reach, with its declared parameters. The list is the table
// itself, so it is current by construction.
func Operations() []api.Op { return api.Ops() }

// Operation looks one operation up by its table name (for example
// "task_get"). The second result is false for a name the table does
// not declare.
func Operation(name string) (api.Op, bool) {
	for _, op := range api.Ops() {
		if op.Name == name {
			return op, true
		}
	}
	return api.Op{}, false
}

// ErrUnknownOperation is returned (wrapped) when Call names an
// operation the table does not declare.
var ErrUnknownOperation = errors.New("client: unknown operation")

// ArgError reports an argument that does not fit the operation's
// declared parameters: an undeclared name, a missing required
// parameter, or a value outside a declared enum. It is a client-side
// refusal — no request was sent.
type ArgError struct {
	Op     string
	Param  string
	Reason string
}

func (e *ArgError) Error() string {
	return fmt.Sprintf("client: operation %s, parameter %s: %s", e.Op, e.Param, e.Reason)
}

// HTTPError is a non-2xx answer from the daemon, with the daemon's
// error envelope decoded. Body keeps the raw answer for the routes
// whose failure payload is itself information.
type HTTPError struct {
	Status    int
	Code      string
	Message   string
	RequestID string
	Body      []byte
}

func (e *HTTPError) Error() string {
	if e.Message != "" {
		return fmt.Sprintf("client: http %d: %s", e.Status, e.Message)
	}
	return fmt.Sprintf("client: http %d: %s", e.Status, strings.TrimSpace(string(e.Body)))
}

// Call performs the named operation with the given arguments and, when
// out is non-nil, decodes the JSON answer into it. Arguments are keyed
// by the parameter names the table declares; where each travels is the
// table's business, not the caller's.
func (c *Client) Call(ctx context.Context, name string, args map[string]any, out any) error {
	body, err := c.CallRaw(ctx, name, args)
	if err != nil {
		return err
	}
	if out == nil || len(body) == 0 {
		return nil
	}
	if err := json.Unmarshal(body, out); err != nil {
		return fmt.Errorf("client: operation %s: decoding response: %w", name, err)
	}
	return nil
}

// Invoke is Call with the answer as a return value, for callers that
// prefer generics over an out parameter.
func Invoke[T any](ctx context.Context, c *Client, name string, args map[string]any) (T, error) {
	var out T
	err := c.Call(ctx, name, args, &out)
	return out, err
}

// CallRaw performs the named operation and returns the raw answer
// body. A non-2xx answer is an *HTTPError.
func (c *Client) CallRaw(ctx context.Context, name string, args map[string]any) ([]byte, error) {
	op, ok := Operation(name)
	if !ok {
		return nil, fmt.Errorf("%w: %s", ErrUnknownOperation, name)
	}
	req, err := c.buildRequest(ctx, op, args)
	if err != nil {
		return nil, err
	}
	resp, err := c.hc.Do(req)
	if err != nil {
		return nil, fmt.Errorf("client: operation %s: %w", name, err)
	}
	defer func() { _ = resp.Body.Close() }()
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("client: operation %s: reading response: %w", name, err)
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return nil, decodeHTTPError(resp.StatusCode, body)
	}
	return body, nil
}

// buildRequest turns an operation declaration plus arguments into the
// HTTP request the daemon expects. Every placement decision — path
// substitution, query encoding, body fields, a raw body — comes from
// the declaration.
func (c *Client) buildRequest(ctx context.Context, op api.Op, args map[string]any) (*http.Request, error) {
	declared := make(map[string]api.Param, len(op.Params))
	for _, p := range op.Params {
		declared[p.Name] = p
	}
	for key := range args {
		if _, ok := declared[key]; !ok {
			return nil, &ArgError{Op: op.Name, Param: key, Reason: "not declared for this operation"}
		}
	}

	path := op.Path
	query := url.Values{}
	bodyFields := map[string]any{}
	var rawBody []byte
	var hasBody bool

	for _, p := range op.Params {
		v, present := args[p.Name]
		if !present || v == nil {
			if p.Required {
				return nil, &ArgError{Op: op.Name, Param: p.Name, Reason: "required but not supplied"}
			}
			continue
		}
		if err := checkEnum(op, p, v); err != nil {
			return nil, err
		}
		switch p.In {
		case api.InPath:
			path = strings.ReplaceAll(path, "{"+p.Name+"}", url.PathEscape(stringValue(v)))
		case api.InQuery:
			for _, s := range queryValues(v) {
				query.Add(p.Name, s)
			}
		case api.InBody:
			if p.Raw {
				b, err := rawBytes(v)
				if err != nil {
					return nil, &ArgError{Op: op.Name, Param: p.Name, Reason: err.Error()}
				}
				rawBody = b
			} else {
				bodyFields[p.Name] = v
			}
			hasBody = true
		default:
			return nil, &ArgError{Op: op.Name, Param: p.Name, Reason: fmt.Sprintf("declared with unknown placement %q", p.In)}
		}
	}
	if strings.Contains(path, "{") {
		return nil, &ArgError{Op: op.Name, Param: path, Reason: "path parameter not substituted"}
	}

	var rdr io.Reader
	contentType := ""
	switch {
	case rawBody != nil:
		rdr = bytes.NewReader(rawBody)
		contentType = "application/octet-stream"
	case hasBody:
		b, err := json.Marshal(bodyFields)
		if err != nil {
			return nil, fmt.Errorf("client: operation %s: encoding body: %w", op.Name, err)
		}
		rdr = bytes.NewReader(b)
		contentType = "application/json"
	}

	u := c.base + path
	if len(query) > 0 {
		u += "?" + query.Encode()
	}
	req, err := http.NewRequestWithContext(ctx, op.Method, u, rdr)
	if err != nil {
		return nil, fmt.Errorf("client: operation %s: %w", op.Name, err)
	}
	if contentType != "" {
		req.Header.Set("Content-Type", contentType)
	}
	if c.token != "" {
		req.Header.Set("Authorization", "Bearer "+c.token)
	}
	return req, nil
}

// decodeHTTPError reads the daemon's error envelope ({error, code,
// message, requestId}, the shape internal/cli's client decodes) into
// an *HTTPError, falling back to the raw body when it is not that
// shape.
func decodeHTTPError(status int, body []byte) *HTTPError {
	e := &HTTPError{Status: status, Body: body}
	var env struct {
		Error     string `json:"error"`
		Code      string `json:"code"`
		Message   string `json:"message"`
		RequestID string `json:"requestId"`
	}
	if json.Unmarshal(body, &env) == nil {
		e.Code = env.Code
		e.RequestID = env.RequestID
		e.Message = env.Message
		if e.Message == "" {
			e.Message = env.Error
		}
	}
	return e
}

// checkEnum refuses a value outside the parameter's declared enum, so
// a misspelling fails here with the allowed values named instead of at
// the daemon as an opaque refusal.
func checkEnum(op api.Op, p api.Param, v any) error {
	if len(p.Enum) == 0 {
		return nil
	}
	s := stringValue(v)
	for _, allowed := range p.Enum {
		if s == allowed {
			return nil
		}
	}
	return &ArgError{
		Op:     op.Name,
		Param:  p.Name,
		Reason: fmt.Sprintf("value %q is not one of %s", s, strings.Join(p.Enum, ", ")),
	}
}

// stringValue renders a scalar argument the way a path segment or a
// query value carries it.
func stringValue(v any) string {
	switch t := v.(type) {
	case string:
		return t
	case json.Number:
		return t.String()
	default:
		return fmt.Sprint(v)
	}
}

// queryValues flattens an argument into query values: a slice repeats
// the parameter, anything else is one value.
func queryValues(v any) []string {
	rv := reflect.ValueOf(v)
	if rv.Kind() == reflect.Slice || rv.Kind() == reflect.Array {
		out := make([]string, 0, rv.Len())
		for i := 0; i < rv.Len(); i++ {
			out = append(out, stringValue(rv.Index(i).Interface()))
		}
		return out
	}
	return []string{stringValue(v)}
}

// rawBytes accepts the shapes a raw body arrives in. Anything else is
// a caller error: silently JSON-encoding a raw body would put bytes on
// the wire the daemon does not expect.
func rawBytes(v any) ([]byte, error) {
	switch t := v.(type) {
	case []byte:
		return t, nil
	case json.RawMessage:
		return []byte(t), nil
	case string:
		return []byte(t), nil
	case io.Reader:
		return io.ReadAll(t)
	default:
		return nil, fmt.Errorf("raw body must be []byte, string, json.RawMessage, or io.Reader, not %T", v)
	}
}
