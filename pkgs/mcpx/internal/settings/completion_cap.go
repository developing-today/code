package settings

// SpecCompletionCap is CompleteResult.values' maxItems in every MCP
// revision: a fact of the protocol, not a default. It lives here so
// the daemon and MCP surfaces share one constant (#2878).
const SpecCompletionCap = 100

// EffectiveCompletionCap resolves a completion.maxValues setting to
// the cap a reply must actually honour: a setting in
// [1, SpecCompletionCap] lowers the cap; anything else — unset/zero,
// negative, or over the spec limit — resolves to SpecCompletionCap.
// Both completion surfaces (daemon /v1/complete and the MCP server)
// compute their cap here so they cannot drift apart (#2878).
func EffectiveCompletionCap(n int) int {
	if n < 1 || n > SpecCompletionCap {
		return SpecCompletionCap
	}
	return n
}
