package codegen

import (
	"encoding/json"
	"fmt"
	"sort"
	"strconv"
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
	b.WriteString("ToolError := [TransportError(Str), StatusError(U16), DaemonError(Str), ToolResultError(Str), EncodingError(Str)]\n\n")
	fmt.Fprintf(&b, "default_endpoint = %s\ndefault_session = %s\n\n", rocString(endpoint), rocString(session))
	b.WriteString("call! : Str => Try(Response.Response, ToolError)\n")
	b.WriteString("call! = |body| {\n")
	b.WriteString("    endpoint = match Env.var_str!(\"MCPX_ENDPOINT\") {\n        Ok(value) => value\n        Err(_) => default_endpoint\n    }\n")
	b.WriteString("    session = match Env.var_str!(\"MCPX_SESSION\") {\n        Ok(value) => value\n        Err(_) => default_session\n    }\n")
	b.WriteString("    token = match Env.var_str!(\"MCPX_TRANSPORT_BEARER_TOKEN\") {\n        Ok(value) => value\n        Err(_) => \"\"\n    }\n")
	b.WriteString("    run = match Env.var_str!(\"MCPX_RUN\") {\n        Ok(value) => value\n        Err(_) => \"\"\n    }\n")
	b.WriteString("    request = Request.from_method(Method.POST)\n        .with_uri(Str.concat(endpoint, \"/v1/call\"))\n        .with_body(Str.to_utf8(body))\n        .add_header(\"Content-Type\", \"application/json\")\n        .add_header(\"x-mcpx-session\", session)\n        .add_header(\"x-mcpx-call\", session)\n        .add_header(\"x-mcpx-run\", run)\n")
	b.WriteString("    authenticated_request = if token == \"\" { request } else { request.add_header(\"Authorization\", Str.concat(\"Bearer \", token)) }\n")
	b.WriteString("    response = Http.send!(authenticated_request, {}) ? |_err| TransportError(\"mcpx daemon request failed\")\n")
	b.WriteString("    if Response.status(response) < 200 or Response.status(response) >= 300 {\n        return Err(StatusError(Response.status(response)))\n    }\n")
	b.WriteString("    response_text = Str.from_utf8(Response.body(response)) ? |_err| TransportError(\"mcpx daemon returned invalid UTF-8\")\n")
	b.WriteString("    envelope : Try({ error ?: Str, result ?: { isError ?: Bool, content ?: List({ type : Str, text ?: Str }) } }, _)\n")
	b.WriteString("    envelope = Json.parse(response_text)\n")
	b.WriteString("    match envelope {\n        Err(_) => Err(DaemonError(\"mcpx daemon returned invalid JSON\"))\n        Ok(value) => match value.?error {\n            Ok(message) => Err(DaemonError(message))\n            Err(_) => match value.?result {\n                Ok(result) => match result.?isError {\n                    Ok(is_error) => if is_error {\n                        blocks = match result.?content {\n                            Ok(content) => List.keep_oks(content, |block| block.?text)\n                            Err(_) => []\n                        }\n                        message = if List.is_empty(blocks) { \"tool returned isError\" } else { Str.join_with(blocks, \"\\n\") }\n                        Err(ToolResultError(message))\n                    } else { Ok(response) }\n                    Err(_) => Ok(response)\n                }\n                Err(_) => Ok(response)\n            }\n        }\n    }\n}\n\n")
	for _, fn := range functions {
		argType := "{ " + strings.Join(fn.fields, ", ") + " }"
		argName := "args"
		if len(fn.args) == 0 {
			argType = "{}"
			argName = "_args"
		}
		where := ""
		if len(fn.genericTypes) > 0 {
			constraints := make([]string, len(fn.genericTypes))
			for i, generic := range fn.genericTypes {
				constraints[i] = generic + ".Json.Encodable(_)"
			}
			where = " where [" + strings.Join(constraints, ", ") + "]"
		}
		fmt.Fprintf(&b, "%s! : %s => Try(Response.Response, ToolError)%s\n", fn.name, argType, where)
		fmt.Fprintf(&b, "%s! = |%s| {\n", fn.name, argName)
		if len(fn.args) == 0 {
			b.WriteString("    args_json = \"{}\"\n")
		} else {
			b.WriteString("    arg_fields = [\n")
		}
		for _, field := range fn.args {
			key, _ := json.Marshal(field.wireName)
			if field.optional {
				fmt.Fprintf(&b, "        match args.?%s {\n            Ok(value) => {\n                encoded = Json.to_str_try(value) ? |_err| EncodingError(\"mcpx argument encoding failed\")\n                Ok(Str.concat(%s, Str.concat(\":\", encoded)))\n            }\n            Err(_) => Ok(\"\")\n        },\n", field.name, strconv.Quote(string(key)))
			} else {
				fmt.Fprintf(&b, "        {\n            encoded = Json.to_str_try(args.%s) ? |_err| EncodingError(\"mcpx argument encoding failed\")\n            Ok(Str.concat(%s, Str.concat(\":\", encoded)))\n        },\n", field.name, strconv.Quote(string(key)))
			}
		}
		if len(fn.args) > 0 {
			b.WriteString("    ]\n    encoded_fields = List.map_try(arg_fields, |entry| entry) ? |_err| EncodingError(\"mcpx argument encoding failed\")\n    present_fields = List.keep_if(encoded_fields, |entry| entry != \"\")\n    args_json = Str.concat(\"{\", Str.concat(Str.join_with(present_fields, \",\"), \"}\"))\n")
		}
		fmt.Fprintf(&b, "    server_json = Json.to_str_try(%s) ? |_err| EncodingError(\"mcpx request encoding failed\")\n", rocString(fn.server))
		fmt.Fprintf(&b, "    tool_json = Json.to_str_try(%s) ? |_err| EncodingError(\"mcpx request encoding failed\")\n", rocString(fn.tool))
		b.WriteString("    body = Str.concat(\"{\\\"server\\\":\", Str.concat(server_json, Str.concat(\",\\\"tool\\\":\", Str.concat(tool_json, Str.concat(\",\\\"args\\\":\", Str.concat(args_json, \"}\"))))))\n    call!(body)\n}\n\n")
	}
	return b.String()
}

type rocFunction struct {
	name, server, tool string
	fields             []string
	args               []rocArgument
	genericTypes       []string
}

type rocArgument struct {
	name, wireName, rocType string
	optional                bool
}

func rocFunctions(nss []Namespace) []rocFunction {
	sorted := append([]Namespace(nil), nss...)
	sort.Slice(sorted, func(i, j int) bool { return sorted[i].Name < sorted[j].Name })
	var out []rocFunction
	usedNames := map[string]struct{}{"call": {}, "default_endpoint": {}, "default_session": {}}
	for _, ns := range sorted {
		for _, tool := range sortedTools(ns) {
			name := uniqueRocIdent(rocIdent(ns.Name+"_"+tool.Name, false), usedNames)
			fields, args := rocInputFields(tool.InputSchema)
			var generics []string
			for i := range args {
				if args[i].rocType == "" {
					args[i].rocType = string(rune('a' + len(generics)))
					generics = append(generics, args[i].rocType)
					for j := range fields {
						fields[j] = strings.Replace(fields[j], "__GENERIC_"+args[i].name+"__", args[i].rocType, 1)
					}
				}
			}
			out = append(out, rocFunction{
				name: name, server: ns.Server,
				tool: tool.Name, fields: fields, args: args, genericTypes: generics,
			})
		}
	}
	return out
}

func rocInputFields(raw json.RawMessage) ([]string, []rocArgument) {
	var schema struct {
		Required   []string `json:"required"`
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
	args := make([]rocArgument, 0, len(keys))
	usedNames := make(map[string]struct{}, len(keys))
	required := make(map[string]struct{}, len(schema.Required))
	for _, k := range schema.Required {
		required[k] = struct{}{}
	}
	for _, k := range keys {
		typ := ""
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
		name := uniqueRocIdent(rocIdent(k, false), usedNames)
		if typ == "" {
			typ = "__GENERIC_" + name + "__"
		}
		_, isRequired := required[k]
		if !isRequired {
			fields = append(fields, name+" ?: "+typ)
		} else {
			fields = append(fields, name+" : "+typ)
		}
		rocType := typ
		if strings.HasPrefix(typ, "__GENERIC_") {
			rocType = ""
		}
		args = append(args, rocArgument{name: name, wireName: k, rocType: rocType, optional: !isRequired})
	}
	if len(fields) == 0 {
		return nil, nil
	}
	return fields, args
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
