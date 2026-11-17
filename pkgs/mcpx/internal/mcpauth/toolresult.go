package mcpauth

// Translating a tools/call outcome into an APIError (#8017).
//
// The failover machinery (#7948) classifies *APIError values, but a
// GitHub API failure inside an MCP session never arrives as one: the
// github server answers the tools/call with an error CallToolResult
// whose text content embeds the API status and body ("... 403 You
// have exceeded a secondary rate limit ..."), or the transport
// surfaces it as a Go error carrying the same text. This file is the
// translation the daemon call path runs before Rotator.Do can see
// the block at all.

import (
	"encoding/json"
	"regexp"
	"strconv"
	"strings"
	"time"
)

// toolResultWire is the CallToolResult shape the translation reads:
// the isError flag and the text content blocks. Structured content is
// deliberately not mined -- a tool that succeeds and returns data
// ABOUT rate limits is not itself rate-limited.
type toolResultWire struct {
	IsError bool `json:"isError"`
	Content []struct {
		Type string `json:"type"`
		Text string `json:"text"`
	} `json:"content"`
}

// ToolResultText extracts the concatenated text blocks and the
// isError flag from a raw CallToolResult. ok is false when raw is
// not a CallToolResult at all.
func ToolResultText(raw json.RawMessage) (text string, isError bool, ok bool) {
	var tr toolResultWire
	if err := json.Unmarshal(raw, &tr); err != nil {
		return "", false, false
	}
	var b strings.Builder
	for _, c := range tr.Content {
		if c.Type != "text" {
			continue
		}
		if b.Len() > 0 {
			b.WriteByte('\n')
		}
		b.WriteString(c.Text)
	}
	return b.String(), tr.IsError, true
}

// statusRe finds an HTTP status the classification understands when
// the text carries one explicitly (the github server embeds it:
// "POST https://api.github.com/...: 403 ...").
var statusRe = regexp.MustCompile(`\b(401|403|404|422|429)\b`)

// retryAfterRe reads a "retry after N" / "retry-after: N" hint, in
// seconds, when the block text carries one.
var retryAfterRe = regexp.MustCompile(`(?i)retry[- ]after[:\s]+(\d+)`)

// APIErrorFromText translates free-form error text -- a tool result's
// text or a Go error's message -- into an *APIError Classify can
// read. It reports false when the text carries neither a recognised
// status nor a soft-block marker: not every failure is a GitHub API
// block, and inventing one would rotate identities on unrelated
// errors. A soft marker with no explicit status is read as 403, the
// status GitHub's primary and secondary limits both arrive under.
func APIErrorFromText(text string) (*APIError, bool) {
	lower := strings.ToLower(text)
	soft := false
	for _, m := range softMarkers {
		if strings.Contains(lower, m) {
			soft = true
			break
		}
	}
	status := 0
	if m := statusRe.FindString(text); m != "" {
		status, _ = strconv.Atoi(m)
	}
	if status == 0 && !soft {
		return nil, false
	}
	if status == 0 {
		status = 403
	}
	ae := &APIError{Status: status, Body: text}
	if m := retryAfterRe.FindStringSubmatch(text); m != nil {
		if s, err := strconv.Atoi(m[1]); err == nil && s > 0 {
			ae.RetryAfter = time.Duration(s) * time.Second
		}
	}
	return ae, true
}

// APIErrorFromToolResult translates an error CallToolResult into an
// *APIError. A successful result is never translated, however
// rate-limit-shaped its content reads: only isError results carry
// blocks.
func APIErrorFromToolResult(raw json.RawMessage) (*APIError, bool) {
	text, isError, ok := ToolResultText(raw)
	if !ok || !isError {
		return nil, false
	}
	return APIErrorFromText(text)
}
