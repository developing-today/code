package settings

// protoSettings govern how mcpx speaks MCP to its own clients.
//
// The knobs here are all about a question an upstream server asked and who
// gets to answer it. That is the one part of the protocol where mcpx cannot
// have a single right answer: a host with a real UI should be asked, a host
// with none should not be, and mcpx cannot tell the difference except by
// what the host declared -- which some declare wrongly.
func protoSettings() []Setting {
	return []Setting{
		{
			Path: "proto.native", Scope: ScopeDaemon, Kind: KindBool, Default: "true",
			Commands: []string{"serve", "daemon"},
			Name:     "Native elicitation",
			Short:    "put an upstream server's questions to mcpx's own MCP client",
			Long: "On, a question raised during a call is offered to the client that " +
				"made the call, by whichever mechanism its protocol revision has: a " +
				"2026-07-28 client gets an input_required result to retry, an older " +
				"one gets elicitation/create on the wire. Only ever to a client that " +
				"declared it can answer. Off, every question goes to the broker and " +
				"its routing policy decides, which is what happened before any " +
				"client could answer one and is still the fallback for every client " +
				"that declared neither elicitation nor sampling.",
		},
		{
			Path: "proto.serveMCP", Scope: ScopeDaemon, Kind: KindBool, Default: "true",
			Commands: []string{"daemon"},
			Name:     "Serve MCP from the daemon",
			Short:    "mount mcpx's own MCP server on the daemon's listeners",
			Long: "The daemon already has a unix socket and a TCP port. Serving MCP " +
				"from a second process meant two HTTP servers with overlapping /v1 " +
				"prefixes, and which one a caller reached decided which half of the " +
				"API existed. Turn this off only to run a daemon that serves /v1 and " +
				"nothing else.",
		},
		{
			Path: "proto.mcpPath", Scope: ScopeDaemon, Kind: KindString, Default: "/mcp",
			Commands: []string{"daemon"},
			Name:     "MCP path", Short: "where the daemon serves MCP",
		},
		{
			Path: "proto.askTimeout", Scope: ScopeDaemon, Kind: KindDuration, Default: "10m",
			Commands: []string{"serve", "daemon"},
			Name:     "Question timeout",
			Short:    "how long one request may be held while a question goes unanswered",
			Long: "A legacy client is blocked for all of it, so it has to sit inside " +
				"whatever that client's own timeout is. A modern client is not " +
				"blocked at all -- it gets its request back and returns -- so the " +
				"only thing this bounds there is how long mcpx waits between rounds.",
		},
		{
			Path: "proto.askPoll", Scope: ScopeDaemon, Kind: KindDuration, Default: "500ms",
			Plumbing: true,
			Name:     "Question poll", Short: "how long one wait for a question may block",
		},
		{
			Path: "proto.askRounds", Scope: ScopeDaemon, Kind: KindInt, Default: "8",
			Commands: []string{"serve", "daemon"},
			Name:     "Question rounds",
			Short:    "how many times one request may come back asking for more",
			Long: "A server that never stops asking is broken or adversarial, and " +
				"without a bound mcpx would relay it forever.",
		},
		{
			Path: "proto.samplingPerMinute", Scope: ScopeDaemon, Kind: KindInt, Default: "60",
			Commands: []string{"daemon"},
			Name:     "Sampling limit per server",
			Short:    "max sampling requests one upstream server may make per minute, across all its sessions",
			Long: "The sampling security considerations say a client SHOULD rate-limit: without a " +
				"bound a buggy or hostile server can make mcpx relay sampling requests -- each " +
				"one a model call wherever it lands -- as fast as it can send them. Over-limit " +
				"requests fail with an error naming this setting. 0 disables the limit.",
		},
		{
			Path: "proto.samplingSessionPerMinute", Scope: ScopeDaemon, Kind: KindInt, Default: "20",
			Commands: []string{"daemon"},
			Name:     "Sampling limit per session",
			Short:    "max sampling requests one upstream server may make within a single session per minute",
			Long: "The per-session companion to proto.samplingPerMinute: one connection's share, " +
				"checked first, so a session refused here does not consume the server's shared " +
				"budget. Over-limit requests fail with an error naming this setting. 0 disables " +
				"the limit.",
		},
		{
			Path: "proto.elicitationPerMinute", Scope: ScopeDaemon, Kind: KindInt, Default: "30",
			Commands: []string{"daemon"},
			Name:     "Elicitation limit per server",
			Short:    "max elicitation requests one upstream server may make per minute, across all its sessions",
			Long: "The elicitation security considerations say a client SHOULD rate-limit: each " +
				"elicitation spends a person's attention, and an unbounded server can flood the " +
				"question broker. Over-limit requests fail with an error naming this setting -- " +
				"never a fabricated cancel, which would claim a person decided. 0 disables the limit.",
		},
		{
			Path: "proto.askTTL", Scope: ScopeDaemon, Kind: KindDuration, Default: "15m",
			Plumbing: true,
			Name:     "Interruptible call lifetime",
			Short:    "how long the daemon keeps a call waiting for an answer",
		},
		{
			Path: "proto.stateTTL", Scope: ScopeDaemon, Kind: KindDuration, Default: "30m",
			Plumbing: true,
			Name:     "Request state lifetime",
			Short:    "how long a client may resume an interrupted request with",
		},
		{
			Path: "proto.sessionIdle", Scope: ScopeDaemon, Kind: KindDuration, Default: "30m",
			Plumbing: true,
			Name:     "MCP session idle",
			Short:    "how long an unused Streamable HTTP session is kept",
			Long: "A session is only ever ended by a DELETE the client may never " +
				"send, so without a sweep the table grows with every host that " +
				"connects once.",
		},
	}
}
