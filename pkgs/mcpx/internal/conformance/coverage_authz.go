package conformance

// Authorization rows judged against the merged #1975 bearer work
// (PR #2711) for #1950's remainder (task 14860). The overrides in
// overrides/authorization.json re-judge these server sides to
// "applies" so the bindings and the recorded gap below -- not the
// catalogue's blanket pointer at closed #1975 -- decide the cells.
func init() {
	register(
		// The token the client presented to the daemon is never
		// passed through: the upstream gets the configured provider
		// key instead.
		S("authz-no-token-passthrough",
			"daemon.TestProxyKeepsUpstreamKeyWhenBearerSet@all"),
		// The daemon accepts only its own configured bearer token,
		// on the /mcp surface as on every route; anything else is a
		// 401 before the MCP handler runs.
		S("authz-accept-only-own-tokens",
			"daemon.TestMCPRejectsUnauthenticatedWhenBearerSet@all"),
	)
	registerGaps(
		// Judged NOT satisfied by the bearer work: PR #2711 gates
		// the HTTP surface and does not touch the stdio credential
		// path. mcpauth's `env` auth type retrieves credentials
		// from the environment for stdio children, but no test
		// traces that retrieval into a stdio child's environment
		// end to end, so the server side stays a gap -- repointed
		// from closed #1975 to live #1950 with this reason.
		Gap{ID: "authz-stdio-use-environment", Side: Server, Issue: "#1950",
			Why: "the merged #1975 bearer work (PR #2711) gates the HTTP surface only; the `env` auth type retrieves credentials from the environment for stdio children (mcpauth) but no test traces that retrieval into a stdio child end to end, so the server-side stdio row is not yet bound -- repointed from closed #1975 to live #1950 (task 14860)"},
	)
}
