package e2e_test

import (
	"bufio"
	"context"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"os/exec"
	"strings"
	"testing"
	"time"
)

// Effect probes for the http/transport settings family (#8469, task
// 20151), following the sibling burns in effect_test.go: every probe
// sets its setting the way a user would (an MCPX_ variable before the
// daemon starts, or a settings-set command for a hot setting), drives
// an ordinary caller, and asserts an observable with/without
// difference. A probe that only read the value back would prove
// nothing: each one below watches a body actually refused, a stream
// actually pinged, a header actually timed out, a token actually
// demanded.

// rawSockDo is sockDo without the JSON assumption: body-limit refusals
// are plain-text errors, and the body is the evidence.
func rawSockDo(t *testing.T, c *http.Client, method, path, body string) (int, string) {
	t.Helper()
	req, err := http.NewRequest(method, "http://mcpx"+path, strings.NewReader(body))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Content-Type", "application/json")
	resp, err := c.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = resp.Body.Close() }()
	b, _ := io.ReadAll(resp.Body)
	return resp.StatusCode, string(b)
}

func getWith(t *testing.T, url string, headers map[string]string) (int, string) {
	t.Helper()
	req, err := http.NewRequest(http.MethodGet, url, nil)
	if err != nil {
		t.Fatal(err)
	}
	for k, v := range headers {
		req.Header.Set(k, v)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = resp.Body.Close() }()
	b, _ := io.ReadAll(resp.Body)
	return resp.StatusCode, string(b)
}

// --- Surface: request bodies the daemon refuses -----------------------

// http.bodyLimit, http.callBodyLimit and http.controlBodyLimit are the
// three MaxBytesReader ceilings on the daemon HTTP surface. Each is
// hot, so one daemon proves all three: at the default a body of a few
// hundred bytes is decoded and answered; with the ceiling set below
// the body, the same request is refused as too large before any
// handler logic runs.
func TestHTTPBodyLimitsRejectOversizedBodies(t *testing.T) {
	e := newEnv(t, oneServer)
	e.run("ls")
	c := e.longSocketClient(t)

	// http.callBodyLimit governs POST /v1/call.
	bigMsg := strings.Repeat("x", 600)
	callBody := "{\"server\":\"demo\",\"tool\":\"echo\",\"args\":{\"message\":\"" + bigMsg + "\"}}"
	if st, body := rawSockDo(t, c, http.MethodPost, "/v1/call", callBody); st != http.StatusOK || !strings.Contains(body, bigMsg) {
		t.Fatalf("oracle: a 600-byte call should succeed at the default callBodyLimit: %d %s", st, body)
	}
	e.run("settings", "set", "http.callBodyLimit", "128")
	if st, body := rawSockDo(t, c, http.MethodPost, "/v1/call", callBody); st != http.StatusBadRequest || !strings.Contains(body, "too large") {
		t.Fatalf("http.callBodyLimit=128 should refuse a 600-byte call body: %d %s", st, body)
	}
	e.run("settings", "set", "http.callBodyLimit", "64MiB")

	// http.bodyLimit governs the ordinary routes, here /v1/resource.
	resBody := "{\"server\":\"demo\",\"uri\":\"file:///tmp/" + strings.Repeat("y", 600) + "\"}"
	if _, body := rawSockDo(t, c, http.MethodPost, "/v1/resource", resBody); strings.Contains(body, "too large") {
		t.Fatalf("oracle: a 600-byte resource body should be decoded at the default bodyLimit: %s", body)
	}
	e.run("settings", "set", "http.bodyLimit", "128")
	if st, body := rawSockDo(t, c, http.MethodPost, "/v1/resource", resBody); st != http.StatusBadRequest || !strings.Contains(body, "too large") {
		t.Fatalf("http.bodyLimit=128 should refuse a 600-byte resource body: %d %s", st, body)
	}
	e.run("settings", "set", "http.bodyLimit", "1MiB")

	// http.controlBodyLimit governs the settings API itself: a PUT
	// whose body carries a large ignored field is the control body.
	padded := "{\"value\":\"3s\",\"pad\":\"" + strings.Repeat("z", 600) + "\"}"
	put := func() (int, string) {
		req, err := http.NewRequest(http.MethodPut, "http://mcpx/v1/settings/http.sseRetry", strings.NewReader(padded))
		if err != nil {
			t.Fatal(err)
		}
		resp, err := c.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		defer func() { _ = resp.Body.Close() }()
		b, _ := io.ReadAll(resp.Body)
		return resp.StatusCode, string(b)
	}
	if st, body := put(); st != http.StatusOK {
		t.Fatalf("oracle: a padded settings PUT should succeed at the default controlBodyLimit: %d %s", st, body)
	}
	e.run("settings", "set", "http.controlBodyLimit", "128")
	if st, body := put(); st != http.StatusBadRequest || !strings.Contains(body, "too large") {
		t.Fatalf("http.controlBodyLimit=128 should refuse a padded settings PUT: %d %s", st, body)
	}
	e.run("settings", "set", "http.controlBodyLimit", "64KiB")
	if st, body := put(); st != http.StatusOK {
		t.Fatalf("restoring http.controlBodyLimit should let the padded PUT through again: %d %s", st, body)
	}
}

// --- Surface: the daemon event stream ---------------------------------

// lineChan turns a streaming response body into a channel of lines.
func lineChan(body io.Reader) chan string {
	lines := make(chan string, 64)
	go func() {
		defer close(lines)
		rd := bufio.NewReader(body)
		for {
			line, err := rd.ReadString('\n')
			if line != "" {
				lines <- strings.TrimRight(line, "\n")
			}
			if err != nil {
				return
			}
		}
	}()
	return lines
}

func nextLine(t *testing.T, lines chan string, within time.Duration) (string, bool) {
	t.Helper()
	select {
	case line, ok := <-lines:
		return line, ok
	case <-time.After(within):
		return "", false
	}
}

// http.sseRetry is the first line of every /v1/events stream: the
// retry hint, in milliseconds. http.ssePing is the comment that keeps
// an idle stream alive. Both are hot, so one daemon shows the default
// and the set value for each.
func TestHTTPEventStreamSettingsChangeTheStream(t *testing.T) {
	e := newEnv(t, oneServer)
	e.run("ls")
	c := e.longSocketClient(t)

	open := func() chan string {
		t.Helper()
		req, err := http.NewRequest(http.MethodGet, "http://mcpx/v1/events", nil)
		if err != nil {
			t.Fatal(err)
		}
		resp, err := c.Do(req)
		if err != nil {
			t.Fatal(err)
		}
		t.Cleanup(func() { _ = resp.Body.Close() })
		if resp.StatusCode != http.StatusOK {
			t.Fatalf("events stream status %d", resp.StatusCode)
		}
		return lineChan(resp.Body)
	}

	lines := open()
	if line, ok := nextLine(t, lines, 5*time.Second); !ok || line != "retry: 2000" {
		t.Fatalf("oracle: the default http.sseRetry should head the stream as retry: 2000, got %q (ok=%v)", line, ok)
	}
	e.run("settings", "set", "http.sseRetry", "123ms")
	lines = open()
	if line, ok := nextLine(t, lines, 5*time.Second); !ok || line != "retry: 123" {
		t.Fatalf("http.sseRetry=123ms should head the stream as retry: 123, got %q (ok=%v)", line, ok)
	}

	// At the default 15s ping no comment arrives promptly; at 200ms
	// one does. The retry line heads both streams and is consumed
	// first, so what is timed here is the wait after it.
	sawPing := func() bool {
		lines := open()
		if _, ok := nextLine(t, lines, 5*time.Second); !ok {
			t.Fatal("the stream should head with a retry line")
		}
		deadline := time.After(2500 * time.Millisecond)
		for {
			select {
			case line, ok := <-lines:
				if !ok {
					return false
				}
				if line == ": ping" {
					return true
				}
			case <-deadline:
				return false
			}
		}
	}
	if sawPing() {
		t.Fatal("oracle: no : ping should arrive within 2.5s at the default http.ssePing of 15s")
	}
	e.run("settings", "set", "http.ssePing", "200ms")
	if !sawPing() {
		t.Fatal("http.ssePing=200ms should put a : ping comment on an idle stream within 2.5s")
	}
}

// --- Surface: raw TCP against the daemon listener ---------------------

// http.readHeaderTimeout is how long a client has to finish sending
// its headers before the server closes the connection. A client that
// sends a partial header block and then nothing is the fixture: at
// the default (10s) its connection is still open seconds later; at
// 300ms the server has already hung up on it.
func TestHTTPReadHeaderTimeoutClosesSlowClients(t *testing.T) {
	hangup := func(env ...string) (bool, time.Duration) {
		t.Helper()
		e := newEnv(t, oneServer)
		e.setenv(env...)
		e.run("ls")
		ep := endpointOf(t, e)
		conn, err := net.DialTimeout("tcp", strings.TrimPrefix(ep, "http://"), 5*time.Second)
		if err != nil {
			t.Fatal(err)
		}
		defer func() { _ = conn.Close() }()
		if _, err := conn.Write([]byte("GET /v1/health HTTP/1.1\r\nHost: localhost\r\n")); err != nil {
			t.Fatal(err)
		}
		start := time.Now()
		_ = conn.SetReadDeadline(time.Now().Add(4 * time.Second))
		_, rerr := conn.Read(make([]byte, 1))
		took := time.Since(start)
		if rerr == nil {
			t.Fatal("a partial-header connection should never be answered")
		}
		if nerr, ok := rerr.(net.Error); ok && nerr.Timeout() {
			return false, took // our deadline, not the server: still open
		}
		return true, took
	}

	closed, took := hangup("MCPX_HTTP_READ_HEADER_TIMEOUT=300ms")
	if !closed || took > 3*time.Second {
		t.Fatalf("http.readHeaderTimeout=300ms should close a partial-header connection promptly, closed=%v after %v", closed, took)
	}
	closed, _ = hangup()
	if closed {
		t.Fatal("oracle: at the default http.readHeaderTimeout a partial-header connection should still be open after 4s")
	}
}

// http.shutdownGrace is how long http.Server.Shutdown waits for
// in-flight requests when the daemon stops. A synchronous /v1/exec
// whose script sleeps 3s is such a request, and it holds no pool
// lease, so the pool drain window cannot mask the grace: with a
// 200ms grace the stop cuts the call and the daemon is gone before
// the script finishes; at the default 5s the call is allowed to
// finish, its answer is delivered, and only then does the stop
// complete.
func TestHTTPShutdownGraceBoundsHowLongAStopWaits(t *testing.T) {
	graceCase := func(env ...string) (bool, time.Duration) {
		t.Helper()
		e := newEnv(t, oneServer)
		e.setenv(env...)
		e.run("ls")
		c := e.longSocketClient(t)
		type result struct {
			st   int
			body string
			err  error
		}
		done := make(chan result, 1)
		go func() {
			src := "{\"source\":\"await new Promise((r) => setTimeout(r, 3000)); console.log(\\\"exec-done\\\")\"}"
			req, err := http.NewRequest(http.MethodPost, "http://mcpx/v1/exec", strings.NewReader(src))
			if err != nil {
				done <- result{err: err}
				return
			}
			req.Header.Set("Content-Type", "application/json")
			resp, err := c.Do(req)
			if err != nil {
				done <- result{err: err}
				return
			}
			defer func() { _ = resp.Body.Close() }()
			b, _ := io.ReadAll(resp.Body)
			done <- result{st: resp.StatusCode, body: string(b)}
		}()
		// The script needs a moment to be in flight inside the
		// daemon; the stop below is what is being measured, and it
		// must not begin before the call it may or may not wait for.
		// The stop command itself returns as soon as the daemon has
		// accepted the shutdown, so what is timed is the call: cut
		// at the grace, or answered when the script finishes.
		time.Sleep(1 * time.Second)
		e.run("stop")
		start := time.Now()
		select {
		case r := <-done:
			return r.err == nil && r.st == http.StatusOK && strings.Contains(r.body, "exec-done"), time.Since(start)
		case <-time.After(10 * time.Second):
			t.Fatal("the exec call neither answered nor was cut within 10s of the stop")
			return false, time.Since(start)
		}
	}

	answered, cutIn := graceCase("MCPX_HTTP_SHUTDOWN_GRACE=200ms")
	if answered {
		t.Fatal("http.shutdownGrace=200ms should cut a 3s in-flight exec when the daemon stops")
	}
	if cutIn > 2*time.Second {
		t.Fatalf("with a 200ms grace the in-flight exec should be cut promptly, took %v", cutIn)
	}
	answered, answeredIn := graceCase()
	if !answered {
		t.Fatal("oracle: at the default http.shutdownGrace of 5s a 3s in-flight exec should finish and answer across a stop")
	}
	if answeredIn < 1*time.Second {
		t.Fatalf("the default grace should hold the daemon until the call finishes (~2s remained), answered in %v", answeredIn)
	}
}

// --- Surface: the CLI as the daemon client ----------------------------

// http.requestTimeout bounds one CLI request to the daemon. The CLI
// is a fresh process per command, so the variable is set on the
// command, not the daemon: a slow tool call that answers at the
// default is abandoned at 300ms with a timeout error.
func TestHTTPRequestTimeoutBoundsAClientCall(t *testing.T) {
	e := newEnv(t, oneServer)
	if out := e.run("call", "demo.slow", "{\"ms\":1200}"); !strings.Contains(out, "slept 1200ms") {
		t.Fatalf("oracle: a 1.2s call should answer at the default http.requestTimeout:\n%s", out)
	}
	short := e.withEnv("MCPX_HTTP_REQUEST_TIMEOUT=300ms")
	start := time.Now()
	out, err := short.try("call", "demo.slow", "{\"ms\":5000}")
	took := time.Since(start)
	if err == nil {
		t.Fatalf("http.requestTimeout=300ms should abandon a 5s call:\n%s", out)
	}
	if took > 4*time.Second {
		t.Fatalf("the call should be abandoned near the timeout, took %v:\n%s", took, out)
	}
	if low := strings.ToLower(out); !strings.Contains(low, "timeout") && !strings.Contains(low, "deadline") {
		t.Fatalf("the abandoned call should say it timed out:\n%s", out)
	}
}

// http.streamBufferMax is the largest single event line the CLI
// events scanner will read: a line over it ends the command with the
// scanners own error. Ordinary event lines are a few hundred bytes,
// so a 256-byte ceiling refuses them, while the default (8MiB)
// delivers the replay and the stream stays open until killed.
func TestHTTPStreamBufferMaxEndsAnOversizedEventLine(t *testing.T) {
	e := newEnv(t, oneServer)
	e.run("call", "demo.echo", "{\"message\":\"x\"}")

	runEvents := func(extra ...string) (string, time.Duration) {
		t.Helper()
		ctx, cancel := context.WithTimeout(context.Background(), 6*time.Second)
		defer cancel()
		cmd := exec.CommandContext(ctx, e.mcpx, "events", "--since", "0")
		cmd.Dir = e.dir
		cmd.Env = append(append([]string{}, e.envVars...), extra...)
		start := time.Now()
		out, _ := cmd.CombinedOutput()
		return string(out), time.Since(start)
	}

	// The initial buffer (http.streamBufferInit, 64KiB by default) is
	// an allocation the scanner may fill before the ceiling is ever
	// consulted, so a ceiling below it cannot bind a short line. The
	// fixture therefore pins the initial buffer at 64 bytes in both
	// phases and varies only the ceiling: what differs between the
	// two runs is http.streamBufferMax alone.
	def, _ := runEvents("MCPX_HTTP_STREAM_BUFFER_INIT=64")
	if !strings.Contains(def, "\"kind\":\"call.finished\"") {
		t.Fatalf("oracle: events at the default http.streamBufferMax should deliver the replayed call:\n%s", def)
	}
	tiny, took := runEvents("MCPX_HTTP_STREAM_BUFFER_INIT=64", "MCPX_HTTP_STREAM_BUFFER_MAX=256")
	if took > 5*time.Second {
		t.Fatalf("http.streamBufferMax=256 should end the events command on the first long line, not ride out the deadline:\n%s", tiny)
	}
	if !strings.Contains(tiny, "token too long") {
		t.Fatalf("the events command should fail with the scanners token-too-long error:\n%s", tiny)
	}
	if strings.Contains(tiny, "\"kind\":\"call.finished\"") {
		t.Fatalf("no event should be delivered past a 256-byte line ceiling:\n%s", tiny)
	}
}

// --- Surface: the daemon /mcp transport -------------------------------

// mcpPost posts one JSON-RPC frame to the daemon /mcp endpoint and
// returns the response frame and headers. The reply may be plain JSON
// or an event stream carrying data lines; both are decoded to the
// last frame.
func mcpRoundTrip(t *testing.T, ep, body, sid string) (map[string]any, http.Header) {
	t.Helper()
	req, err := http.NewRequest(http.MethodPost, ep+"/mcp", strings.NewReader(body))
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "application/json, text/event-stream")
	if sid != "" {
		req.Header.Set("Mcp-Session-Id", sid)
	}
	resp, err := (&http.Client{Timeout: 30 * time.Second}).Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer func() { _ = resp.Body.Close() }()
	raw, _ := io.ReadAll(resp.Body)
	if resp.StatusCode != http.StatusOK {
		t.Fatalf("POST /mcp answered %d: %s", resp.StatusCode, raw)
	}
	var frame map[string]any
	for _, line := range strings.Split(string(raw), "\n") {
		data, ok := strings.CutPrefix(strings.TrimSpace(line), "data:")
		if !ok {
			continue
		}
		var f map[string]any
		if json.Unmarshal([]byte(strings.TrimSpace(data)), &f) == nil {
			frame = f
		}
	}
	if frame == nil {
		if err := json.Unmarshal(raw, &frame); err != nil {
			t.Fatalf("undecodable /mcp reply: %v\n%s", err, raw)
		}
	}
	return frame, resp.Header
}

func mcpInit(t *testing.T, ep string) string {
	t.Helper()
	_, hdr := mcpRoundTrip(t, ep,
		"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{\"name\":\"t\",\"version\":\"1\"}}}", "")
	sid := hdr.Get("Mcp-Session-Id")
	if sid == "" {
		t.Fatal("initialize issued no Mcp-Session-Id")
	}
	return sid
}

// mcpCallText calls one fakemcp tool through the gateway tool and
// returns the text of its result.
func mcpCallText(t *testing.T, ep, sid string, id int, tool string, args map[string]any, meta map[string]any) string {
	t.Helper()
	params := map[string]any{
		"name": "mcpx_call",
		"arguments": map[string]any{
			"namespace": "demo", "tool": tool, "arguments": args,
		},
	}
	if meta != nil {
		params["_meta"] = meta
	}
	frameBody, err := json.Marshal(map[string]any{
		"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": params,
	})
	if err != nil {
		t.Fatal(err)
	}
	frame, _ := mcpRoundTrip(t, ep, string(frameBody), sid)
	b, _ := json.Marshal(frame)
	var decoded struct {
		Result struct {
			Content []struct {
				Text string `json:"text"`
			} `json:"content"`
		} `json:"result"`
		Error any `json:"error"`
	}
	if err := json.Unmarshal(b, &decoded); err != nil || decoded.Error != nil {
		t.Fatalf("tools/call %s failed: %v %v\n%s", tool, err, decoded.Error, b)
	}
	var texts []string
	for _, c := range decoded.Result.Content {
		texts = append(texts, c.Text)
	}
	return strings.Join(texts, "\n")
}

// transport.bearerToken is the shared-secret gate on every /v1 and
// /mcp request. With the token set on the daemon, a raw caller
// without it (or with the wrong one) is refused 401, the right token
// is served, and first-party callers -- the CLI, which reads the same
// setting -- keep working, as the ls that started the daemon already
// proved. Without the setting, the same raw caller is served.
func TestTransportBearerTokenGatesTheDaemon(t *testing.T) {
	token := strings.Repeat("t", 40) // the daemon refuses shorter tokens at startup

	e := newEnv(t, oneServer)
	e.setenv("MCPX_TRANSPORT_BEARER_TOKEN=" + token)
	e.run("ls")
	ep := endpointOf(t, e)
	if code, _ := getWith(t, ep+"/v1/health", nil); code != http.StatusUnauthorized {
		t.Fatalf("no token should be refused 401, got %d", code)
	}
	if code, _ := getWith(t, ep+"/v1/health", map[string]string{"Authorization": "Bearer wrong-" + token}); code != http.StatusUnauthorized {
		t.Fatalf("a wrong token should be refused 401, got %d", code)
	}
	if code, body := getWith(t, ep+"/v1/health", map[string]string{"Authorization": "Bearer " + token}); code != http.StatusOK {
		t.Fatalf("the configured token should be served, got %d: %s", code, body)
	}
	if out := e.run("ls"); !strings.Contains(out, "demo") {
		t.Fatalf("the CLI should keep working against its own tokend daemon:\n%s", out)
	}

	open := newEnv(t, oneServer)
	open.run("ls")
	if code, _ := getWith(t, endpointOf(t, open)+"/v1/health", nil); code != http.StatusOK {
		t.Fatalf("oracle: with no bearerToken set a raw caller should be served, got %d", code)
	}
}

// transport.allowedOrigins is the list of browser origins, beyond
// loopback, that may reach the daemon. The with/without is the
// setting itself: the same origin is refused 403 against a daemon
// started without it and served against one started with it, while a
// never-listed origin is refused by both.
func TestTransportAllowedOriginsAreServed(t *testing.T) {
	verdict := func(env ...string) (int, int) {
		t.Helper()
		e := newEnv(t, oneServer)
		e.setenv(env...)
		e.run("ls")
		ep := endpointOf(t, e)
		src := "{\"source\":\"console.log(ok-+app)\"}"
		configured, _ := postWith(t, ep+"/v1/exec", src,
			map[string]string{"Origin": "https://app.example.com", "Content-Type": "application/json"})
		foreign, _ := postWith(t, ep+"/v1/exec", src,
			map[string]string{"Origin": "https://evil.example", "Content-Type": "application/json"})
		return configured, foreign
	}

	configured, foreign := verdict()
	if configured != http.StatusForbidden || foreign != http.StatusForbidden {
		t.Fatalf("oracle: unlisted origins should be refused, got configured %d foreign %d", configured, foreign)
	}
	configured, foreign = verdict("MCPX_TRANSPORT_ALLOWED_ORIGINS=https://app.example.com")
	if configured != http.StatusOK {
		t.Fatalf("transport.allowedOrigins should serve the listed origin, got %d", configured)
	}
	if foreign != http.StatusForbidden {
		t.Fatalf("an origin that is still unlisted should be refused, got %d", foreign)
	}
}

// transport.sessionMetaKeys is the precedence list of _meta keys a
// client may name its session with. The stateful fixture server is
// session-scoped, so the session a call lands in is visible in the
// state it can see: under the default keys a custom key names
// nothing and two calls share the connection session; once the key
// is configured, two calls naming different values land in different
// sessions and cannot see each others state.
func TestTransportSessionMetaKeysNameTheSession(t *testing.T) {
	e := newEnv(t, statefulServer)
	e.run("ls")
	ep := endpointOf(t, e)
	sid := mcpInit(t, ep)

	meta := func(v string) map[string]any { return map[string]any{"acme/session": v} }
	mcpCallText(t, ep, sid, 2, "open", map[string]any{"value": "first"}, meta("one"))
	if got := mcpCallText(t, ep, sid, 3, "state", map[string]any{}, meta("two")); !strings.Contains(got, "first") {
		t.Fatalf("oracle: under the default keys a custom _meta key should name no session, so the state should be shared: %s", got)
	}

	e.run("settings", "set", "transport.sessionMetaKeys", "acme/session")
	mcpCallText(t, ep, sid, 4, "open", map[string]any{"value": "second"}, meta("three"))
	if got := mcpCallText(t, ep, sid, 5, "state", map[string]any{}, meta("four")); strings.Contains(got, "second") {
		t.Fatalf("transport.sessionMetaKeys=acme/session should put differently-named calls in different sessions: %s", got)
	}
}

// transport.sseKeepAlive is how often a quiet MCP GET stream carries
// a comment. The comment is a bare ":" line: at 200ms one arrives
// promptly on an idle stream; at the 15s default none does.
func TestTransportSSEKeepAliveCommentsOnAQuietStream(t *testing.T) {
	sawComment := func(env ...string) bool {
		t.Helper()
		e := newEnv(t, oneServer)
		e.setenv(env...)
		e.run("ls")
		ep := endpointOf(t, e)
		sid := mcpInit(t, ep)
		req, err := http.NewRequest(http.MethodGet, ep+"/mcp", nil)
		if err != nil {
			t.Fatal(err)
		}
		req.Header.Set("Accept", "text/event-stream")
		req.Header.Set("Mcp-Session-Id", sid)
		resp, err := (&http.Client{Timeout: 10 * time.Second}).Do(req)
		if err != nil {
			t.Fatal(err)
		}
		defer func() { _ = resp.Body.Close() }()
		if resp.StatusCode != http.StatusOK {
			t.Fatalf("GET /mcp stream status %d", resp.StatusCode)
		}
		lines := lineChan(resp.Body)
		deadline := time.After(2500 * time.Millisecond)
		for {
			select {
			case line, ok := <-lines:
				if !ok {
					return false
				}
				if line == ":" {
					return true
				}
			case <-deadline:
				return false
			}
		}
	}

	if !sawComment("MCPX_TRANSPORT_SSE_KEEP_ALIVE=200ms") {
		t.Fatal("transport.sseKeepAlive=200ms should comment on a quiet stream within 2.5s")
	}
	if sawComment() {
		t.Fatal("oracle: at the default transport.sseKeepAlive of 15s no comment should arrive within 2.5s")
	}
}

// transport.stdioDrain is how long a serve process lets an in-flight
// request finish after its stdin closes. The fixture is a slow tool
// call written to a serve process whose input then closes at once:
// with a 5s drain the call answers; with a 100ms drain it is
// cancelled and never answered, and the process exits.
func TestTransportStdioDrainBoundsServeShutdown(t *testing.T) {
	e := newEnv(t, oneServer)

	answered := func(drain string) bool {
		t.Helper()
		cmd := exec.Command(e.mcpx, "serve")
		cmd.Dir = e.dir
		cmd.Env = append(append([]string{}, e.envVars...), "MCPX_TRANSPORT_STDIO_DRAIN="+drain)
		stdin, err := cmd.StdinPipe()
		if err != nil {
			t.Fatal(err)
		}
		stdout, err := cmd.StdoutPipe()
		if err != nil {
			t.Fatal(err)
		}
		if err := cmd.Start(); err != nil {
			t.Fatal(err)
		}
		defer func() { _ = cmd.Process.Kill(); _ = cmd.Wait() }()
		lines := lineChan(stdout)
		write := func(s string) {
			if _, err := stdin.Write([]byte(s + "\n")); err != nil {
				t.Fatal(err)
			}
		}
		write("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{\"name\":\"t\",\"version\":\"1\"}}}")
		for {
			line, ok := nextLine(t, lines, 20*time.Second)
			if !ok {
				t.Fatal("serve closed before answering initialize")
			}
			if strings.Contains(line, "\"id\":1") {
				break
			}
		}
		write("{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}")
		write("{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"mcpx_call\",\"arguments\":{\"namespace\":\"demo\",\"tool\":\"slow\",\"arguments\":{\"ms\":1500}}}}")
		_ = stdin.Close()
		for {
			line, ok := nextLine(t, lines, 15*time.Second)
			if !ok {
				return false
			}
			if strings.Contains(line, "\"id\":2") {
				return true
			}
		}
	}

	if !answered("5s") {
		t.Fatal("transport.stdioDrain=5s should let a 1.5s in-flight call answer after stdin closes")
	}
	if answered("100ms") {
		t.Fatal("transport.stdioDrain=100ms should cancel a 1.5s in-flight call when stdin closes, not answer it")
	}
}

// http.streamBufferMax also bounds a streamed remote run: `mcpx exec` with
// exec.where=remote and exec.output=stream reads the daemon's NDJSON frames
// with the same configured ceiling `mcpx events` uses. It used the built-in
// 8MiB whatever the setting said, so a lower ceiling never bound it.
func TestHTTPStreamBufferMaxBoundsAStreamedRemoteRun(t *testing.T) {
	e := newEnv(t, oneServer)
	e.setenv("MCPX_EXEC_WHERE=remote", "MCPX_EXEC_OUTPUT=stream", "MCPX_HTTP_STREAM_BUFFER_INIT=64")
	script := `console.log("y".repeat(4000))`
	if out, err := e.try("exec", script); err != nil || strings.Count(out, "y") < 4000 {
		t.Fatalf("oracle: at the default http.streamBufferMax a 4000-byte line should stream back whole (err %v):\n%s", err, firstN(out, 300))
	}
	out, err := e.withEnv("MCPX_HTTP_STREAM_BUFFER_MAX=1024").try("exec", script)
	if err == nil || !strings.Contains(out, "token too long") {
		t.Fatalf("http.streamBufferMax=1024 should end a streamed run whose frame is longer (err %v):\n%s", err, firstN(out, 300))
	}
}
