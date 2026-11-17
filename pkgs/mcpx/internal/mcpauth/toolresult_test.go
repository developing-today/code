package mcpauth

import (
	"encoding/json"
	"testing"
	"time"
)

// Oracle tests for the #8017 translation: the oracle is Classify's
// contract -- a translated soft block must rotate (BlockSoft), a
// translated hard block must not (BlockHard), and anything that is
// not a GitHub API block must not translate at all.

func TestAPIErrorFromToolResultSoftBlockRotates(t *testing.T) {
	raw := json.RawMessage(`{"isError":true,"content":[{"type":"text","text":"failed to create issue: POST https://api.github.com/repos/o/r/issues: 403 You have exceeded a secondary rate limit. Please wait a few minutes before you try again."}]}`)
	ae, ok := APIErrorFromToolResult(raw)
	if !ok {
		t.Fatal("oracle: secondary-limit tool result did not translate")
	}
	if ae.Status != 403 {
		t.Errorf("oracle: status = %d, want 403", ae.Status)
	}
	if got := Classify(ae); got != BlockSoft {
		t.Errorf("oracle: Classify = %v, want BlockSoft (rotate)", got)
	}
}

func TestAPIErrorFromToolResult429Rotates(t *testing.T) {
	raw := json.RawMessage(`{"isError":true,"content":[{"type":"text","text":"429 Too Many Requests: rate limit exceeded, retry after 30"}]}`)
	ae, ok := APIErrorFromToolResult(raw)
	if !ok {
		t.Fatal("oracle: 429 tool result did not translate")
	}
	if ae.Status != 429 {
		t.Errorf("oracle: status = %d, want 429", ae.Status)
	}
	if ae.RetryAfter != 30*time.Second {
		t.Errorf("oracle: RetryAfter = %v, want 30s", ae.RetryAfter)
	}
	if got := Classify(ae); got != BlockSoft {
		t.Errorf("oracle: Classify = %v, want BlockSoft (rotate)", got)
	}
}

func TestAPIErrorFromToolResultHardBlockDoesNotRotate(t *testing.T) {
	raw := json.RawMessage(`{"isError":true,"content":[{"type":"text","text":"403 Forbidden: resource not accessible by integration"}]}`)
	ae, ok := APIErrorFromToolResult(raw)
	if !ok {
		t.Fatal("oracle: marker-less 403 tool result did not translate")
	}
	if got := Classify(ae); got != BlockHard {
		t.Errorf("oracle: Classify = %v, want BlockHard (surface unchanged)", got)
	}
}

func TestAPIErrorFromToolResultSuccessNeverTranslates(t *testing.T) {
	// A tool that succeeds returning documentation ABOUT rate limits
	// carries the soft markers in ordinary content; rotating on it
	// would fail over healthy calls.
	raw := json.RawMessage(`{"content":[{"type":"text","text":"docs: when you hit a rate limit, GitHub returns 403 and you submitted too quickly errors follow"}]}`)
	if ae, ok := APIErrorFromToolResult(raw); ok {
		t.Errorf("oracle: successful result translated to %v; want no translation", ae)
	}
}

func TestAPIErrorFromTextUnrelatedErrorDoesNotTranslate(t *testing.T) {
	if ae, ok := APIErrorFromText("connection reset by peer"); ok {
		t.Errorf("oracle: unrelated error translated to %v; want no translation", ae)
	}
	if ae, ok := APIErrorFromText("boom: deliberate failure"); ok {
		t.Errorf("oracle: unrelated tool failure translated to %v; want no translation", ae)
	}
}
