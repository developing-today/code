package settings

import (
	"strconv"

	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
	"github.com/developing-today/code/pkgs/mcpx/internal/tasks"
)

// protoTasksSettings govern the tasks mcpx hands out, over MCP and /v1 alike.
func protoTasksSettings() []Setting {
	return []Setting{
		{
			Path: "protoTasks.pollInterval", Scope: ScopeDaemon, Kind: KindDuration,
			Default:  defaults.Str(defaults.TaskPollInterval),
			Commands: []string{"serve", "daemon"},
			Name:     "Task poll interval",
			Short:    "the pollInterval every task carries",
			Long: "How often a client is told it may usefully ask after a task. The " +
				"specification says a client SHOULD respect it, so it is the rate " +
				"mcpx is asking to be polled at. It was a literal one second in the " +
				"task store.",
		},
		{
			Path: "protoTasks.maxTTL", Scope: ScopeDaemon, Kind: KindDuration,
			Default:  tasks.DefaultMaxTTL.String(),
			Commands: []string{"serve", "daemon"},
			Name:     "Task maximum TTL",
			Short:    "the longest TTL a task may ask for; longer requests are clamped",
			Long: "Store.Start clamps every requested TTL to this bound, on the MCP " +
				"and /v1 surfaces alike, so a task result cannot be retained for " +
				"longer than the deployment allows (#2498, Phase 1 of #2497).",
		},
		{
			Path: "protoTasks.maxPerRequestor", Scope: ScopeDaemon, Kind: KindInt,
			Default:  strconv.Itoa(tasks.DefaultMaxPerRequestor),
			Commands: []string{"serve", "daemon"},
			Name:     "Task cap per requestor",
			Short:    "how many non-terminal tasks one requestor may hold at once",
			Long: "Store.Start refuses a new task with a named error once the " +
				"requestor already holds this many non-terminal tasks; the " +
				"unbound bucket (a caller with no identity) is capped the same " +
				"way (#2498, Phase 1 of #2497).",
		},
		{
			Path: "protoTasks.perMinute", Scope: ScopeDaemon, Kind: KindInt,
			Default:  strconv.Itoa(tasks.DefaultPerMinute),
			Commands: []string{"serve", "daemon"},
			Name:     "Tasks per minute per requestor",
			Short:    "the tasks/* admission window per requestor; 0 disables it",
			Long: "Every tasks/* call (tasks/result included — counted at " +
				"admission, so a long poll is not punished for waiting) counts " +
				"against this fixed per-minute window for its requestor; past " +
				"it the call is refused (#2498, Phase 1 of #2497).",
		},
	}
}
