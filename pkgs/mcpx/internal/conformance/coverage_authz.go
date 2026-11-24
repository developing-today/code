package conformance

// Authorization rows judged against the merged #1975 bearer <redacted>
// (PR #2711) for #1950's remainder (task 14860). The overrides in
// overrides/authorization.json re-judge these server sides to
// "applies" so the bindings below -- not the catalogue's blanket
// pointer at closed #1975 -- decide the cells.
func init() {
	register(
		// The token the client presented to the daemon is never
		// passed through: the upstream gets the configured provider
		// key instead.
		S("authz-no-token-passthrough",
			"daemon.TestProxyKeepsUpstreamKeyWhenBearerSet@all"),
		// The daemon accepts only its own configured bearer <redacted>,
		// on the /mcp surface as on every route; anything else is a
		// 401 before the MCP handler runs.
		S("authz-accept-only-own-tokens",
			"daemon.TestMCPRejectsUnauthenticatedWhenBearerSet@all"),
		// A stdio server does not run the OAuth flow: its
		// credential is retrieved from the daemon's environment by
		// the `env` auth type (mcpauth) and carried into the
		// spawned child. The pool test traces that retrieval end
		// to end -- the child itself reports the value it
		// received, under a name set nowhere but the auth block.
		S("authz-stdio-use-environment",
			"pool.TestStdioChildReceivesEnvCredential@all"),
	)
}
