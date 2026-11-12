package api

import (
	"fmt"
	"strings"

	"github.com/developing-today/code/pkgs/mcpx/internal/refdoc"
)

// Markdown renders the /v1 operation table as a Markdown reference page
// (#4909, remainder of #1846 slice 3).
//
// Generated from Ops() — the same table OpenAPI renders and the daemon
// routes — so the page cannot describe an operation that does not exist
// or omit one that does. It renders the static operation table only:
// nothing here reads a running daemon, whose /v1/openapi.json carries
// that builder's configured tools and is a different document by design
// (see the comment on cli.OpenAPI).
func Markdown(version string) string {
	ops := Ops()
	var b strings.Builder
	b.WriteString("# The /v1 API\n\n")
	fmt.Fprintf(&b, "mcpx %s. %d operations, generated from the operation table "+
		"the daemon routes and `mcpx openapi` renders. The static specifications "+
		"are `openapi.json` (this surface alone) and `openapi-serve.json` "+
		"(the serve surface: this surface plus the MCP endpoint, one REST path "+
		"per MCP tool, and the settings index) in the same directory.\n\n", version, len(ops))

	headers := []string{"operation", "route", "summary", "parameters", "notes"}
	var rows [][]string
	for _, op := range ops {
		var params []string
		for _, p := range op.Params {
			where := string(p.In)
			if p.Raw {
				params = append(params, fmt.Sprintf("`%s` (raw body)", p.Name))
				continue
			}
			req := ""
			if p.Required || p.In == InPath {
				req = ", required"
			}
			params = append(params, fmt.Sprintf("`%s` (%s%s)", p.Name, where, req))
		}
		var notes []string
		if op.Admin {
			notes = append(notes, "privileged")
		}
		if op.Destructive {
			notes = append(notes, "destructive")
		}
		if op.Streams {
			notes = append(notes, "streams; no MCP tool — MCP carries it as `subscriptions/listen`")
		} else if op.NoTool != "" {
			notes = append(notes, "no MCP tool: "+op.NoTool)
		} else {
			notes = append(notes, "MCP tool `"+op.ToolName()+"`")
		}
		rows = append(rows, []string{
			"`" + op.Name + "`",
			"`" + op.Method + " " + op.Path + "`",
			op.Summary,
			strings.Join(params, "<br>"),
			strings.Join(notes, "; "),
		})
	}
	b.WriteString(refdoc.Table(headers, rows))
	b.WriteString("\n## Operations in detail\n")
	for _, op := range ops {
		fmt.Fprintf(&b, "\n### `%s`\n\n`%s %s` — %s\n\n%s\n", op.Name, op.Method, op.Path, op.Summary, op.Description)
		if len(op.Params) > 0 {
			ph := []string{"parameter", "in", "required", "description"}
			var pr [][]string
			for _, p := range op.Params {
				req := "no"
				if p.Required || p.In == InPath {
					req = "yes"
				}
				pr = append(pr, []string{"`" + p.Name + "`", string(p.In), req, p.Desc})
			}
			// Declaration order: parameter order is part of the
			// declaration, so the rows are not sorted.
			for _, p := range op.Params {
				req := "no"
				if p.Required || p.In == InPath {
					req = "yes"
				}
				pr = append(pr, []string{"`" + p.Name + "`", string(p.In), req, p.Desc})
			}
			b.WriteString("\n")
			b.WriteString(refdoc.Table(ph, pr))
		}
	}
	return b.String()
}
