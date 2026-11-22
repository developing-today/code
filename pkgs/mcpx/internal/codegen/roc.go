package codegen

import (
	"encoding/json"
	"fmt"
	"sort"
	"strings"
)

// Roc is the Roc basic-cli target. Generated clients use the HTTP API exposed
// by the selected basic-cli platform and the shared roc-lang/http types.
var Roc = Language{
	Name: "roc", ClientFile: "Mcpx.roc", ScriptExt: ".roc",
	ContentType: "text/x-roc", module: RocModule,
	globals:      func([]Namespace) string { return "" },
	declarations: RocDeclarations,
}

// RocModule emits a client module for Roc's basic-cli platform. The host
// supplies basic-cli as the app platform; roc-lang/http is also listed as a
// direct app package because generated modules import its shared HTTP types.
// Tool argument records are concrete per-tool types so Roc can statically
// select its JSON encoder for each request.
func RocModule(nss []Namespace, endpoint, session string) string {
	var b strings.Builder
	functions := rocFunctions(nss)
	// Roc's current module format is a headerless type module. The file is
	// written as Mcpx.roc, so define the matching top-level type.
	b.WriteString("Mcpx := {}\n\n")
	b.WriteString("import pf.Http\nimport pf.Env\nimport http.Method\nimport http.Request\nimport http.Response\n\n")
	b.WriteString("ToolError := [TransportError(Str), StatusError(U16)]\n\n")
	fmt.Fprintf(&b, "default_endpoint = %s\ndefault_session = %s\n\n", rocString(endpoint), rocString(session))
	b.WriteString("call! : Str => Try(Response.Response, ToolError)\n")
	b.WriteString("call! = |body| {\n")
	b.WriteString("    endpoint = match Env.var_str!(\"MCPX_ENDPOINT\") {\n        Ok(value) => value\n        Err(_) => default_endpoint\n    }\n")
	b.WriteString("    session = match Env.var_str!(\"MCPX_SESSION\") {\n        Ok(value) => value\n        Err(_) => default_session\n    }\n")
	b.WriteString("    request = Request.from_method(Method.POST)\n        .with_uri(Str.concat(endpoint, \"/v1/call\"))\n        .with_body(Str.to_utf8(body))\n        .add_header(\"Content-Type\", \"application/json\")\n        .add_header(\"x-mcpx-session\", session)\n        .add_header(\"x-mcpx-call\", session)\n")
	b.WriteString("    response = Http.send!(request, {}) ? |_err| TransportError(\"mcpx daemon request failed\")\n")
	b.WriteString("    if Response.status(response) < 200 or Response.status(response) >= 300 {\n        return Err(StatusError(Response.status(response)))\n    }\n    Ok(response)\n}\n\n")
	for _, fn := range functions {
		fmt.Fprintf(&b, "%s! : { %s } => Try(Response.Response, ToolError)\n", fn.name, strings.Join(fn.fields, ", "))
		fmt.Fprintf(&b, "%s! = |args| {\n", fn.name)
		fmt.Fprintf(&b, "    payload = { server: %s, tool: %s, args }\n", rocString(fn.server), rocString(fn.tool))
		b.WriteString("    body = Json.to_str_try(payload) ? |_err| TransportError(\"mcpx request encoding failed\")\n    call!(body)\n}\n\n")
	}
	return b.String()
}

type rocFunction struct {
	name, server, tool string
	fields             []string
}

func rocFunctions(nss []Namespace) []rocFunction {
	sorted := append([]Namespace(nil), nss...)
	sort.Slice(sorted, func(i, j int) bool { return sorted[i].Name < sorted[j].Name })
	var out []rocFunction
	usedNames := map[string]struct{}{"call": {}, "default_endpoint": {}, "default_session": {}}
	for _, ns := range sorted {
		for _, tool := range sortedTools(ns) {
			name := uniqueRocIdent(rocIdent(ns.Name+"_"+tool.Name, false), usedNames)
			fields := rocInputFields(tool.InputSchema)
			out = append(out, rocFunction{
				name: name, server: ns.Server,
				tool: tool.Name, fields: fields,
			})
		}
	}
	return out
}

func rocInputFields(raw json.RawMessage) []string {
	var schema struct {
		Properties map[string]struct {
			Type string `json:"type"`
		} `json:"properties"`
	}
	_ = json.Unmarshal(raw, &schema)
	keys := make([]string, 0, len(schema.Properties))
	for k := range schema.Properties {
		keys = append(keys, k)
	}
	sort.Strings(keys)
	fields := make([]string, 0, len(keys))
	usedNames := make(map[string]struct{}, len(keys))
	for _, k := range keys {
		typ := "Json.Value"
		switch schema.Properties[k].Type {
		case "string":
			typ = "Str"
		case "integer":
			typ = "I64"
		case "number":
			typ = "F64"
		case "boolean":
			typ = "Bool"
		}
		fields = append(fields, uniqueRocIdent(rocIdent(k, false), usedNames)+" : "+typ)
	}
	if len(fields) == 0 {
		return []string{"_unit : {}"}
	}
	return fields
}

func uniqueRocIdent(name string, used map[string]struct{}) string {
	candidate := name
	for suffix := 2; ; suffix++ {
		if _, exists := used[candidate]; !exists {
			used[candidate] = struct{}{}
			return candidate
		}
		candidate = fmt.Sprintf("%s_%d", name, suffix)
	}
}

func RocDeclarations(nss []Namespace) string {
	var b strings.Builder
	b.WriteString("Roc client (basic-cli platform)\n\n")
	for _, fn := range rocFunctions(nss) {
		fmt.Fprintf(&b, "%s : generated typed arguments -> Try(Response.Response, ToolError)\n", fn.name)
	}
	return b.String()
}

func rocString(s string) string {
	var b strings.Builder
	b.WriteByte('"')
	for _, r := range s {
		switch r {
		case '\\':
			b.WriteString("\\\\")
		case '"':
			b.WriteString("\\\"")
		case '\n':
			b.WriteString("\\n")
		case '\r':
			b.WriteString("\\r")
		case '\t':
			b.WriteString("\\t")
		default:
			if r < 0x20 {
				fmt.Fprintf(&b, "\\u(%04x)", r)
			} else {
				b.WriteRune(r)
			}
		}
	}
	b.WriteByte('"')
	return b.String()
}

var rocReserved = map[string]struct{}{
	"and": {}, "app": {}, "as": {}, "crash": {}, "dbg": {}, "else": {}, "expect": {},
	"exposes": {}, "for": {}, "if": {}, "implements": {}, "import": {}, "in": {}, "is": {},
	"match": {}, "module": {}, "not": {}, "or": {}, "packages": {}, "platform": {},
	"provides": {}, "requires": {}, "return": {}, "test": {}, "then": {}, "try": {},
	"when": {}, "where": {}, "with": {},
}

func rocIdent(s string, upper bool) string {
	var b strings.Builder
	for i, r := range s {
		if r >= 'A' && r <= 'Z' || r >= 'a' && r <= 'z' || r >= '0' && r <= '9' || r == '_' {
			if i == 0 && r >= '0' && r <= '9' {
				b.WriteByte('_')
			}
			b.WriteRune(r)
		} else {
			b.WriteByte('_')
		}
	}
	name := strings.Trim(b.String(), "_")
	if name == "" {
		name = "field"
	}
	if upper {
		name = strings.ToUpper(name[:1]) + name[1:]
	} else {
		name = strings.ToLower(name[:1]) + name[1:]
	}
	if _, reserved := rocReserved[name]; reserved {
		name += "_"
	}
	return name
}
