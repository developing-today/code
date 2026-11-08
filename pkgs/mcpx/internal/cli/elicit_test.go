package cli

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"strings"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/elicit"
)

// #1678 (Phase 3 of #1675): a URL question shows the registrable domain
// under the full URL, so a subdomain spoof has to survive a second line.
func TestRenderQuestionURLShowsRegistrableDomain(t *testing.T) {
	out := renderQuestion(elicit.Request{
		ID: "elc-1", Message: "Log in", Mode: elicit.URL,
		URL: "https://login.example.co.uk/auth?x=1",
	})
	if !strings.Contains(out, "  url      https://login.example.co.uk/auth?x=1\n") {
		t.Errorf("full URL missing or changed:\n%s", out)
	}
	if !strings.Contains(out, "  domain   example.co.uk\n") {
		t.Errorf("registrable domain line missing:\n%s", out)
	}
	if strings.Contains(out, "warn") {
		t.Errorf("plain ASCII host must not warn:\n%s", out)
	}
}

// #1678: a punycode (or raw non-ASCII) host prints both forms plus the
// warning line, in the settled wording.
func TestRenderQuestionURLWarnsOnPunycodeHost(t *testing.T) {
	for _, raw := range []string{"https://xn--mnchen-3ya.de/login", "https://münchen.de/login"} {
		out := renderQuestion(elicit.Request{
			ID: "elc-2", Message: "Log in", Mode: elicit.URL, URL: raw,
		})
		if !strings.Contains(out, "  host     münchen.de (xn--mnchen-3ya.de)\n") {
			t.Errorf("%s: both host forms missing:\n%s", raw, out)
		}
		if !strings.Contains(out, "  warn     host uses punycode; check the domain is the one you expect\n") {
			t.Errorf("%s: warn line missing:\n%s", raw, out)
		}
	}
}

// #1678: schema defaults appear in the field detail and in the answer
// example; fields without one keep the "..." placeholder.
func TestRenderQuestionFormShowsSchemaDefaults(t *testing.T) {
	schema := json.RawMessage(`{"type":"object","properties":{
		"repo":{"type":"string","default":"me/thing"},
		"count":{"type":"number","default":3},
		"note":{"type":"string"}},"required":["repo"]}`)
	out := renderQuestion(elicit.Request{
		ID: "elc-3", Message: "Details", Mode: elicit.Form, Schema: schema,
	})
	if !strings.Contains(out, `default: "me/thing"`) {
		t.Errorf("string default missing from field detail:\n%s", out)
	}
	if !strings.Contains(out, "default: 3") {
		t.Errorf("number default missing from field detail:\n%s", out)
	}
	if !strings.Contains(out, `"repo":"me/thing"`) || !strings.Contains(out, `"count":3`) {
		t.Errorf("defaults missing from answer example:\n%s", out)
	}
	if !strings.Contains(out, `"note":"..."`) {
		t.Errorf("defaultless field lost its placeholder:\n%s", out)
	}
}

// #1678: the answer review masks credential-shaped values as set/not set
// and never prints the value itself.
func TestRenderAnswerReviewMasksSensitiveFields(t *testing.T) {
	r := elicit.Request{
		Mode: elicit.Form,
		Schema: json.RawMessage(`{"type":"object","properties":{
			"pw":{"type":"string","format":"password"},
			"api_token":{"type":"string"},
			"repo":{"type":"string"}}}`),
	}
	out := renderAnswerReview(r, json.RawMessage(
		`{"api_token":"hunter2","pw":"s3cret","repo":"me/thing","empty_secret":""}`))
	for _, leaked := range []string{"hunter2", "s3cret"} {
		if strings.Contains(out, leaked) {
			t.Errorf("review leaked a credential value:\n%s", out)
		}
	}
	if !strings.Contains(out, "api_token") || !strings.Contains(out, "(set)") {
		t.Errorf("sensitive field not masked as (set):\n%s", out)
	}
	if !strings.Contains(out, "empty_secret") || !strings.Contains(out, "(not set)") {
		t.Errorf("empty sensitive field not masked as (not set):\n%s", out)
	}
	if !strings.Contains(out, "me/thing") {
		t.Errorf("plain field value missing from review:\n%s", out)
	}
}

// The relay decision defaults to no: only an explicit y/yes sends, and a
// vanished terminal (EOF) never does.
func TestConfirmSendRequiresExplicitYes(t *testing.T) {
	for in, want := range map[string]bool{
		"y\n": true, "yes\n": true, "Y\n": true,
		"n\n": false, "\n": false, "": false, "send it\n": false,
	} {
		got, err := confirmSend(context.Background(), strings.NewReader(in))
		if err != nil || got != want {
			t.Errorf("confirmSend(%q) = %v, %v; want %v", in, got, err, want)
		}
	}
}

// Codex P1 on PR 3011: a schema default containing an apostrophe (or a
// shell injection attempt) must survive shell-quoting in the suggested
// answer command: the JSON is one shell word, embedded quotes escaped.
func TestRenderQuestionShellQuotesDefaults(t *testing.T) {
	schema := json.RawMessage(`{"type":"object","properties":{
		"name":{"type":"string","default":"O'Reilly"},
		"evil":{"type":"string","default":"'; printf INJECTED; #"}}}`)
	out := renderQuestion(elicit.Request{
		ID: "elc-4", Message: "Details", Mode: elicit.Form, Schema: schema,
	})
	if !strings.Contains(out, `'{"evil":"'\''; printf INJECTED; #","name":"O'\''Reilly"}'`) {
		t.Errorf("suggested command does not shell-quote the whole JSON as one word:\n%s", out)
	}
}

// Codex P2 on PR 3011: the answer review shows the effective answer --
// schema defaults for omitted fields included, since the relay applies
// them -- with sensitive defaults still masked.
func TestRenderAnswerReviewIncludesEffectiveDefaults(t *testing.T) {
	r := elicit.Request{
		Mode: elicit.Form,
		Schema: json.RawMessage(`{"type":"object","properties":{
			"share_data":{"type":"boolean","default":true},
			"api_token":{"type":"string","format":"password","default":"d3fault"},
			"repo":{"type":"string"}}}`),
	}
	out := renderAnswerReview(r, json.RawMessage(`{"repo":"me/thing"}`))
	if !strings.Contains(out, "share_data") || !strings.Contains(out, "true") {
		t.Errorf("omitted field's default missing from review:\n%s", out)
	}
	if strings.Contains(out, "d3fault") {
		t.Errorf("review leaked a sensitive default value:\n%s", out)
	}
	if !strings.Contains(out, "api_token") || !strings.Contains(out, "(set)") {
		t.Errorf("sensitive default not masked as (set) in review:\n%s", out)
	}
}

// Codex P2 on PR 3011: URL hosts are case-insensitive but the
// publicsuffix list is not -- an uppercase host must still resolve to
// the registrable domain, not render the suffix itself.
func TestRenderQuestionURLUppercaseHostShowsRegistrableDomain(t *testing.T) {
	out := renderQuestion(elicit.Request{
		ID: "elc-5", Message: "Log in", Mode: elicit.URL,
		URL: "https://login.example.CO.UK/auth",
	})
	if !strings.Contains(out, "  domain   example.co.uk\n") {
		t.Errorf("uppercase host lost its registrable domain:\n%s", out)
	}
}

// Codex P2 on PR 3011: cancelling the command context (Ctrl-C) while
// the confirmation read is blocked must end the prompt without a
// relay decision, not leave the command stuck on stdin.
func TestConfirmSendHonoursContextCancellation(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	pr, pw := io.Pipe()
	defer pw.Close()
	defer pr.Close()
	done := make(chan error, 1)
	go func() {
		ok, err := confirmSend(ctx, pr)
		if ok {
			done <- errors.New("confirmSend returned yes on a cancelled read")
			return
		}
		done <- err
	}()
	cancel()
	select {
	case err := <-done:
		if !errors.Is(err, context.Canceled) {
			t.Errorf("confirmSend on cancel = %v; want context.Canceled", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("confirmSend ignored context cancellation and stayed blocked")
	}
}
