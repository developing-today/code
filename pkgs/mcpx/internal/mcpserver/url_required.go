package mcpserver

import (
	"encoding/json"
)

// codeURLElicitationRequired is -32042, UrlElicitationRequired: the
// 2025-11-25 error a server returns when a call is blocked by URL-mode
// elicitations the host must complete out of band before a retry
// (#2080, Phase 3 of #2077). 2026-07-28 removed the type and reserves
// the code, so it is emitted at 2025-11-25 only -- never to a modern
// peer, never in any other configuration.
const codeURLElicitationRequired = -32042

// eligibleURLElicitationRequired reports whether p is a host -32042
// may be sent to: it negotiated exactly 2025-11-25 (the only revision
// that defines the error) and declared elicitation.url. A modern peer
// is excluded by construction -- its version is a 2026 one -- and the
// explicit Modern check keeps that true even for a hand-built Peer.
func (p Peer) eligibleURLElicitationRequired() bool {
	return !p.Modern && p.Version == "2025-11-25" && p.CanElicit("url")
}

// urlQuestionsOf filters pending questions to the URL-mode
// elicitations among them: exactly what a -32042 may list. Form
// questions, sampling and roots are never listed -- a form question
// the host cannot be asked stays with the broker, as before.
func urlQuestionsOf(qs []Question) []Question {
	var out []Question
	for _, q := range qs {
		if q.Method != "elicitation/create" {
			continue
		}
		mode := q.Mode
		if mode == "" {
			var pm struct {
				Mode string `json:"mode"`
			}
			_ = json.Unmarshal(q.Params, &pm)
			mode = pm.Mode
		}
		if mode == "url" {
			out = append(out, q)
		}
	}
	return out
}

// urlRequiredResponse renders the -32042 answer for a call blocked by
// the URL questions in qs, or nil when this peer may not be sent one
// or no URL question is pending. data.elicitations is exactly those
// questions, each rendered via paramsFor -- which is what synthesises
// the required elicitationId ("mcpx-<id>") where the upstream sent
// none. The caller returns the response WITHOUT abandoning the call:
// the broker keeps the questions, the host completes them out of
// band and retries, mirroring the client-side flow of #2079.
func urlRequiredResponse(req request, p Peer, qs []Question) *response {
	urls := urlQuestionsOf(qs)
	if !p.eligibleURLElicitationRequired() || len(urls) == 0 {
		return nil
	}
	els := make([]json.RawMessage, 0, len(urls))
	for _, q := range urls {
		params, err := q.paramsFor(p)
		if err != nil {
			params = q.Params
		}
		els = append(els, params)
	}
	data, err := json.Marshal(map[string]any{"elicitations": els})
	if err != nil {
		return nil
	}
	return &response{JSONRPC: "2.0", ID: req.ID, Error: &rpcError{
		Code:    codeURLElicitationRequired,
		Message: "url elicitation required",
		Data:    json.RawMessage(data),
	}}
}
