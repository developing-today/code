// Package mcpx is the module root: it carries what belongs to the module
// as a whole rather than to any one internal package.
//
// AGENTS.md is this module's agent-facing documentation. It lives beside
// this file so an agent working in the source tree finds it where AGENTS.md
// files are expected; embedding it here is also the only place go:embed can
// reach it -- embed patterns cannot climb out of the embedding package's own
// directory, and every other mcpx package sits below this one.
package mcpx

import _ "embed"

// AgentDocs is the verbatim content of AGENTS.md, embedded in the binary so
// `mcpx help agents` can print version-matched agent documentation to an
// agent that has the binary but neither the repository nor a network. It is
// the file itself, not a copy: editing AGENTS.md changes what the next
// build prints.
//
//go:embed AGENTS.md
var AgentDocs string
