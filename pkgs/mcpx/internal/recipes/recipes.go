// Package recipes turns saved scripts into parameterised, matchable units.
//
// A recipe is not a new artefact. It is a script that already exists on the
// scripts search path, which happens to declare the holes in itself. That
// matters for the reason the issue gives: repeat work should be deterministic
// and free, and the cheapest way to make it so is to reuse what somebody
// already wrote and got working rather than to generate it again.
//
// Declaration is a header comment, because a comment survives every runtime,
// cannot affect execution, and is visible in the file somebody edits:
//
//	// Close stale issues in a repository.
//	// @param repo:string        which repository, as owner/name
//	// @param days:number = 30   how old counts as stale
//	// @param dryRun:boolean?    report without closing
//
//	const stale = await demo.search({ repo: @repo, olderThanDays: @days });
//
// References in the body are `@name`, the same spelling the launcher uses for
// its own holes, and they are replaced by the JSON encoding of the value. A
// recipe is therefore still a valid script to read, and an invalid one to run
// until its holes are filled -- which is the right way round: the file cannot
// be run by accident with a placeholder left in it.
package recipes

import (
	"encoding/json"
	"fmt"
	"math"
	"path/filepath"
	"regexp"
	"sort"
	"strconv"
	"strings"

	"github.com/developing-today/code/pkgs/mcpx/internal/diagnose"
)

// Type is the type of a placeholder value.
type Type string

const (
	TypeString  Type = "string"
	TypeNumber  Type = "number"
	TypeBoolean Type = "boolean"
	// TypeJSON is anything else: an object, an array, a value the recipe
	// wants to pass through untouched.
	TypeJSON Type = "json"
)

// Placeholder is one parameter of a recipe.
type Placeholder struct {
	Name        string `json:"name"`
	Type        Type   `json:"type"`
	Description string `json:"description,omitempty"`
	// Default is the literal as written in the declaration. It is validated
	// against the type when the recipe is rendered. Absent means there is none.
	Default *string `json:"default,omitempty"`
	// Required is false when a default was given or the name was marked
	// optional with a trailing question mark.
	Required bool `json:"required"`
}

// Recipe is a saved script with its holes described.
type Recipe struct {
	Name string `json:"name"`
	Path string `json:"path"`
	// Summary is the first comment line, as `mcpx scripts` shows it.
	Summary      string        `json:"summary,omitempty"`
	Placeholders []Placeholder `json:"placeholders,omitempty"`
	// Tools are the namespace.tool calls the body makes, which is what makes
	// matching free text to a recipe more than a name comparison.
	Tools []string `json:"tools,omitempty"`
	// OptOut is set by a `@tool false` header. Recipe tools are exposed in
	// tools/list by default -- the schema is already written down, so the
	// useful question is how to hide one, not how to publish one.
	OptOut bool `json:"optOut,omitempty"`
	// Source is the file's contents. Omitted from listings, which would
	// otherwise be dominated by it.
	Source string `json:"source,omitempty"`
}

// paramDecl matches the head of one `@param name:type[?] [= default]
// description` line, leaving the tail to be taken apart by hand -- a default
// may be a quoted string or a brace-delimited literal, and a regular
// expression that got that right would be unreadable.
//
// The type is optional and defaults to string, because most placeholders are
// one, and making every declaration carry `:string` would be ceremony that
// teaches nothing.
var paramDecl = regexp.MustCompile(
	`^\s*(?://|#|\*)\s*@param\s+([A-Za-z_][A-Za-z0-9_]*)(\?)?(?:\s*:\s*([A-Za-z]+))?(\?)?\s*(.*)$`)

// declLine recognises a declaration without taking it apart, so the body
// scanner can skip one. Without it the `@param` of the declaration itself
// reads as a placeholder called "param".
var declLine = regexp.MustCompile(`^\s*(?://|#|\*)\s*@param\b`)

// jsonNumberLiteral recognizes the complete JSON number grammar. ParseFloat
// accepts additional Go spellings (such as Inf and hexadecimal floats) that
// are not valid JavaScript/JSON numeric literals.
var jsonNumberLiteral = regexp.MustCompile(`^-?(?:0|[1-9][0-9]*)(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+)?$`)

// toolDecl matches the MCP exposure marker, `@tool` on its own (on, the
// default) or `@tool false` (off). A recipe is a tool unless its author says
// otherwise: the schema already exists, and a repository with fifty scripts
// is exactly the case the budget in `mcpx catalog` answers.
var toolDecl = regexp.MustCompile(`^\s*(?://|#|\*)\s*@tool\b\s*([A-Za-z0-9]*)`)

// Parse reads a script and describes it as a recipe.
func Parse(name, path, src string) Recipe {
	r := Recipe{Name: name, Path: path, Source: src}
	var declared []Placeholder
	seen := map[string]bool{}

	for _, line := range strings.Split(src, "\n") {
		trimmed := strings.TrimSpace(line)
		if r.Summary == "" && strings.HasPrefix(trimmed, "//") &&
			!strings.Contains(trimmed, "@param") && !strings.Contains(trimmed, "@tool") {
			r.Summary = strings.TrimSpace(strings.TrimPrefix(trimmed, "//"))
		}
		m := paramDecl.FindStringSubmatch(line)
		if m == nil {
			if t := toolDecl.FindStringSubmatch(line); t != nil {
				switch strings.ToLower(t[1]) {
				case "false", "off", "no", "0":
					r.OptOut = true
				}
			}
			continue
		}
		p := Placeholder{Name: m[1], Type: normaliseType(m[3]), Required: true}
		if m[2] == "?" || m[4] == "?" {
			p.Required = false
		}
		rest := strings.TrimSpace(m[5])
		if after, ok := strings.CutPrefix(rest, "="); ok {
			// `= 30 how old counts as stale`: the default is the first
			// token unless the value is quoted or bracketed, in which case
			// it runs to its closing mark, and the rest is the description.
			value, tail := splitDefault(strings.TrimSpace(after))
			p.Default = &value
			p.Required = false
			rest = strings.TrimSpace(tail)
		}
		// A `--` before the description is accepted and not required: some
		// people write one, and insisting either way would be a rule whose
		// only effect is to make a declaration silently lose its prose.
		p.Description = strings.TrimSpace(strings.TrimPrefix(rest, "--"))
		if seen[p.Name] {
			continue
		}
		seen[p.Name] = true
		declared = append(declared, p)
	}

	used := refs(src)
	// A name used in the body but never declared is still a placeholder:
	// refusing to run it would be right, but describing it as a required
	// string is more useful, because the form then asks for it.
	for _, name := range used {
		if seen[name] {
			continue
		}
		seen[name] = true
		declared = append(declared, Placeholder{Name: name, Type: TypeString, Required: true})
	}
	// Declared but unused placeholders stay: a recipe may pass one through
	// to a tool by another route, and dropping it would make the form
	// disagree with the file.
	r.Placeholders = declared

	toolSet := map[string]bool{}
	for _, c := range diagnose.Calls(src) {
		if jsGlobals[c.Namespace] {
			continue
		}
		toolSet[c.Path()] = true
	}
	for t := range toolSet {
		r.Tools = append(r.Tools, t)
	}
	sort.Strings(r.Tools)
	return r
}

// jsGlobals are the built-in objects whose members look exactly like tool
// calls. This is a fact about JavaScript rather than a setting, which is why
// it is a list here and not a knob: nobody wants `console.log` counted as a
// tool, and nobody wants to configure that.
//
// Getting it wrong is cheap in one direction and not the other. A namespace
// missing from a recipe's tool list only weakens its ranking; `console.log`
// present in one makes every recipe match the word "log".
var jsGlobals = map[string]bool{
	"console": true, "JSON": true, "Math": true, "Object": true, "Array": true,
	"String": true, "Number": true, "Boolean": true, "Promise": true,
	"Date": true, "RegExp": true, "Map": true, "Set": true, "Error": true,
	"globalThis": true, "process": true, "Deno": true, "Bun": true,
	"window": true, "document": true, "localStorage": true, "Reflect": true,
	"Symbol": true, "BigInt": true, "Intl": true, "WebAssembly": true,
}

func normaliseType(s string) Type {
	switch strings.ToLower(s) {
	case "number", "int", "integer", "float":
		return TypeNumber
	case "bool", "boolean":
		return TypeBoolean
	case "json", "object", "array", "any":
		return TypeJSON
	}
	return TypeString
}

// splitDefault takes the literal off the front of the text after `=`.
func splitDefault(s string) (value, tail string) {
	if s == "" {
		return "", ""
	}
	switch s[0] {
	case '"', '\'', '`':
		q := s[0]
		for i := 1; i < len(s); i++ {
			if s[i] == '\\' {
				i++
				continue
			}
			if s[i] == q {
				return s[:i+1], s[i+1:]
			}
		}
		return s, ""
	case '{', '[':
		open, close := s[0], byte('}')
		if open == '[' {
			close = ']'
		}
		depth := 0
		for i := 0; i < len(s); i++ {
			// A bracket inside a quoted string is text, not
			// structure: without string state, {"note": "a}b"}
			// was truncated at the } inside the string, and the
			// invalid fragment became the default (#1671). Skip
			// strings the way the quoted branch above does,
			// honouring backslash escapes.
			if s[i] == '"' || s[i] == '\'' || s[i] == '`' {
				q := s[i]
				for i++; i < len(s); i++ {
					if s[i] == '\\' {
						i++
						continue
					}
					if s[i] == q {
						break
					}
				}
				continue
			}
			switch s[i] {
			case open:
				depth++
			case close:
				depth--
				if depth == 0 {
					return s[:i+1], s[i+1:]
				}
			}
		}
		return s, ""
	}
	if i := strings.IndexAny(s, " \t"); i >= 0 {
		return s[:i], s[i:]
	}
	return s, ""
}

// scanCode marks, for every byte of src, whether that byte sits in code
// position: outside every comment and string literal. refs and substitute
// share the one scan, so the two can never disagree about whether an
// `@name` is a placeholder -- the defect behind #1670 was each of them
// scanning statelessly while this file's own docs promised comment and
// string skipping.
//
// The scan is lexical, carried across lines so block comments and
// template literals span them, and deliberately small: line comments,
// block comments, and single-, double- and backtick-quoted literals with
// backslash escapes. Two known edges follow from that size and are
// accepted rather than parsed around: a regex literal is not told apart
// from division, so an `@name` after a `//`-shaped regex is read as a
// comment; and `${...}` inside a template literal is treated as string
// text, not as the interpolation it is.
func scanCode(src string) []bool {
	const (
		stCode = iota
		stLineComment
		stBlockComment
		stSingle
		stDouble
		stTemplate
	)
	code := make([]bool, len(src))
	state := stCode
	for i := 0; i < len(src); i++ {
		if state == stCode && (i == 0 || src[i-1] == '\n') {
			// A declaration or marker line is metadata, not script.
			// refs and Render skip such lines for their own
			// purposes; the scan must skip them too, or their prose
			// is lexed as JavaScript and a /* in a @param
			// description opens a block comment over the body.
			// Only a line the scanner reaches in code position is
			// neutralized -- a lookalike line inside a comment or
			// string is content, and its bytes still count.
			line := src[i:]
			if j := strings.IndexByte(line, '\n'); j >= 0 {
				line = line[:j]
			}
			if declLine.MatchString(line) || toolDecl.MatchString(line) {
				i += len(line) - 1 // the loop step lands on the newline
				continue
			}
		}
		c := src[i]
		switch state {
		case stCode:
			switch {
			case c == '/' && i+1 < len(src) && src[i+1] == '/':
				state = stLineComment
				i++
			case c == '/' && i+1 < len(src) && src[i+1] == '*':
				state = stBlockComment
				i++
			case c == '\'':
				state = stSingle
			case c == '"':
				state = stDouble
			case c == '`':
				state = stTemplate
			default:
				code[i] = true
			}
		case stLineComment:
			// JavaScript ends a line comment at every line
			// terminator -- LF, CR, U+2028 and U+2029 -- not just
			// LF: code after a standalone CR is code again, and
			// scanning it as comment text left its placeholders
			// undiscovered and unsubstituted (Codex P2 on #2957).
			switch {
			case c == '\n' || c == '\r':
				state = stCode
				code[i] = true
			case c == 0xE2 && i+2 < len(src) && src[i+1] == 0x80 && (src[i+2] == 0xA8 || src[i+2] == 0xA9):
				state = stCode
				code[i] = true
			}
		case stBlockComment:
			if c == '*' && i+1 < len(src) && src[i+1] == '/' {
				state = stCode
				i++
			}
		case stSingle, stDouble:
			quote := byte('"')
			if state == stSingle {
				quote = '\''
			}
			switch c {
			case '\\':
				i++ // the escaped byte is string text, whatever it is
				if i < len(src) && src[i] == '\r' && i+1 < len(src) && src[i+1] == '\n' {
					// A CRLF continuation is one escaped newline:
					// escaping only the \r would leave the \n to
					// end the string below and desync the scan.
					i++
				}
			case quote:
				state = stCode
			case '\n':
				// An unterminated literal ends at the line, so one bad
				// string cannot swallow the rest of the script.
				state = stCode
				code[i] = true
			}
		case stTemplate:
			switch c {
			case '\\':
				i++
			case '`':
				state = stCode
			}
		}
	}
	return code
}

// refs finds every `@name` in the body, ignoring declarations, comments and
// anything inside a string.
func refs(src string) []string {
	code := scanCode(src)
	var out []string
	seen := map[string]bool{}
	offset := 0
	for _, line := range strings.Split(src, "\n") {
		if declLine.MatchString(line) || toolDecl.MatchString(line) {
			// Declaration and marker lines describe the recipe; the
			// `@param`/`@tool` words in them are not placeholders.
			offset += len(line) + 1
			continue
		}
		for i := 0; i < len(line); i++ {
			if line[i] != '@' || !code[offset+i] {
				continue
			}
			// An email address or a decorator is not a placeholder, and the
			// cheapest way to tell is that a placeholder is preceded by
			// whitespace or punctuation rather than by a word character.
			if i > 0 && isWordByte(line[i-1]) {
				continue
			}
			j := i + 1
			for j < len(line) && isWordByte(line[j]) {
				j++
			}
			if j == i+1 {
				continue
			}
			name := line[i+1 : j]
			if !seen[name] {
				seen[name] = true
				out = append(out, name)
			}
			i = j - 1
		}
		offset += len(line) + 1
	}
	sort.Strings(out)
	return out
}

func isWordByte(b byte) bool {
	return b == '_' || b == '$' ||
		(b >= 'a' && b <= 'z') || (b >= 'A' && b <= 'Z') || (b >= '0' && b <= '9')
}

// ExposesTool says whether this recipe belongs in tools/list.
//
// Default on: a recipe already carries a typed schema, so hiding it should
// take a line somebody wrote. `@tool false` is that line.
func (r Recipe) ExposesTool() bool { return !r.OptOut }

// Missing lists the placeholders a set of values does not supply and that
// have no default to fall back on.
func (r Recipe) Missing(values map[string]any) []Placeholder {
	var out []Placeholder
	for _, p := range r.Placeholders {
		if _, given := values[p.Name]; given {
			continue
		}
		if p.Default != nil || !p.Required {
			continue
		}
		out = append(out, p)
	}
	return out
}

// Render substitutes values into the body.
//
// Values are written as literals of the body's own language, so a
// string arrives quoted and an object arrives as an object.
// Substituting raw text would make every recipe an injection site,
// and a recipe is run with the user's credentials. The language is
// the recipe's own (see language): JavaScript gets JSON literals,
// Python gets Python literals (True/False/None -- Codex P2 on PR
// #8317), and shell gets single-quoted words, inside which the shell
// performs no substitution at all, so a value like $(touch /tmp/x)
// stays data (Codex P1 on PR #8317).
func (r Recipe) Render(values map[string]any) (string, error) {
	lang := r.language()
	lits := map[string]string{}
	for _, p := range r.Placeholders {
		v, given := values[p.Name]
		switch {
		case given:
			lit, err := literalFor(lang, p, v)
			if err != nil {
				return "", err
			}
			lits[p.Name] = lit
		case p.Default != nil:
			v, ok := decodeLiteralFor(lang, p, *p.Default)
			if !ok {
				return "", fmt.Errorf("%s default must be a valid %s: %q", p.Name, p.Type, *p.Default)
			}
			lit, err := literalFor(lang, p, v)
			if err != nil {
				return "", fmt.Errorf("%s default must be a valid %s: %w", p.Name, p.Type, err)
			}
			lits[p.Name] = lit
		case !p.Required:
			lits[p.Name] = undefinedLiteral(lang)
		default:
			return "", fmt.Errorf("recipe %s needs %s", r.Name, p.Name)
		}
	}

	code := scanCodeFor(r.Source, lang)
	var b strings.Builder
	offset := 0
	for _, line := range strings.Split(r.Source, "\n") {
		if declLine.MatchString(line) {
			// The declaration is consumed. Leaving it in would be harmless
			// but the rendered script is what a diagnostic reports line
			// numbers against, and they should match what ran. The
			// consumed line keeps the declaration's own comment marker:
			// rewriting a #-style (Python/shell) declaration with //
			// produced a line that is not a comment in that language
			// and broke the rendered script (PR #8317, #1933 slice 2).
			trimmed := strings.TrimSpace(line)
			marker := "//"
			if strings.HasPrefix(trimmed, "#") {
				marker = "#"
			} else if strings.HasPrefix(trimmed, "*") {
				marker = "*"
			}
			b.WriteString(marker + strings.TrimPrefix(trimmed, marker) + "\n")
			offset += len(line) + 1
			continue
		}
		b.WriteString(substitute(line, code[offset:offset+len(line)], lits))
		b.WriteString("\n")
		offset += len(line) + 1
	}
	return strings.TrimRight(b.String(), "\n") + "\n", nil
}

// substitute replaces `@name` placeholders on one line. code is the
// scanCode mask for exactly this line's bytes: an `@name` outside code
// position (in a comment or a string) is copied through untouched, so a
// filled value is never spliced into a literal.
func substitute(line string, code []bool, lits map[string]string) string {
	var b strings.Builder
	for i := 0; i < len(line); i++ {
		if line[i] != '@' || !code[i] || (i > 0 && isWordByte(line[i-1])) {
			b.WriteByte(line[i])
			continue
		}
		j := i + 1
		for j < len(line) && isWordByte(line[j]) {
			j++
		}
		name := line[i+1 : j]
		lit, ok := lits[name]
		if !ok {
			b.WriteByte(line[i])
			continue
		}
		b.WriteString(lit)
		i = j - 1
	}
	return b.String()
}

// literal renders one value as a JavaScript literal; literalFor is
// the language-aware form Render uses.
func literal(p Placeholder, v any) (string, error) { return literalFor(langJS, p, v) }

func literalFor(lang string, p Placeholder, v any) (string, error) {
	switch p.Type {
	case TypeNumber:
		switch n := v.(type) {
		case int:
			return strconv.Itoa(n), nil
		case int8:
			return strconv.FormatInt(int64(n), 10), nil
		case int16:
			return strconv.FormatInt(int64(n), 10), nil
		case int32:
			return strconv.FormatInt(int64(n), 10), nil
		case int64:
			return strconv.FormatInt(n, 10), nil
		case uint:
			return strconv.FormatUint(uint64(n), 10), nil
		case uint8:
			return strconv.FormatUint(uint64(n), 10), nil
		case uint16:
			return strconv.FormatUint(uint64(n), 10), nil
		case uint32:
			return strconv.FormatUint(uint64(n), 10), nil
		case uint64:
			return strconv.FormatUint(n, 10), nil
		case uintptr:
			return strconv.FormatUint(uint64(n), 10), nil
		case float32:
			if math.IsNaN(float64(n)) || math.IsInf(float64(n), 0) {
				return "", fmt.Errorf("%s is a number, got %q", p.Name, v)
			}
			return strconv.FormatFloat(float64(n), 'g', -1, 32), nil
		case float64:
			if math.IsNaN(n) || math.IsInf(n, 0) {
				return "", fmt.Errorf("%s is a number, got %q", p.Name, v)
			}
			return strconv.FormatFloat(n, 'g', -1, 64), nil
		case string:
			return numberStringLiteral(p, n)
		case json.Number:
			return numberStringLiteral(p, n.String())
		default:
			return "", fmt.Errorf("%s is a number, got %q", p.Name, v)
		}
	case TypeBoolean:
		switch t := v.(type) {
		case bool:
			return boolLiteral(lang, t), nil
		case string:
			b, err := strconv.ParseBool(t)
			if err != nil {
				return "", fmt.Errorf("%s is a boolean, got %q", p.Name, t)
			}
			return boolLiteral(lang, b), nil
		default:
			return "", fmt.Errorf("%s is a boolean, got %q", p.Name, v)
		}
	case TypeString:
		if _, ok := v.(string); !ok {
			return "", fmt.Errorf("%s is a string, got %q", p.Name, v)
		}
	case TypeJSON:
		if s, ok := v.(string); ok && json.Valid([]byte(s)) {
			switch lang {
			case langPython:
				var parsed any
				if err := json.Unmarshal([]byte(s), &parsed); err == nil {
					return pythonValue(parsed)
				}
				return s, nil
			case langShell:
				return shellQuote(s), nil
			}
			return s, nil
		}
	}
	return valueLiteral(lang, p, v)
}

func numberStringLiteral(p Placeholder, n string) (string, error) {
	if !jsonNumberLiteral.MatchString(n) {
		return "", fmt.Errorf("%s is a number, got %q", p.Name, n)
	}
	if _, err := strconv.ParseFloat(n, 64); err != nil {
		return "", fmt.Errorf("%s is a number, got %q", p.Name, n)
	}
	return n, nil
}

// FormSchema is the elicitation schema that asks for the missing values.
//
// Every field carries its description and, where there is one, its default,
// because the broker's contract is that a question can be answered without a
// person: a headless caller times out and the defaults are what remains.
func FormSchema(ps []Placeholder) json.RawMessage {
	props := map[string]any{}
	var required []string
	for _, p := range ps {
		f := map[string]any{}
		if jt := jsonType(p.Type); jt != "" {
			f["type"] = jt
		}
		if p.Description != "" {
			f["description"] = p.Description
		}
		if p.Default != nil {
			if v, ok := decodeLiteral(p, *p.Default); ok {
				f["default"] = v
			}
		}
		props[p.Name] = f
		if p.Required && p.Default == nil {
			required = append(required, p.Name)
		}
	}
	sort.Strings(required)
	doc := map[string]any{"type": "object", "properties": props}
	if len(required) > 0 {
		doc["required"] = required
	}
	b, _ := json.Marshal(doc)
	return b
}

func jsonType(t Type) string {
	switch t {
	case TypeNumber:
		return "number"
	case TypeBoolean:
		return "boolean"
	case TypeJSON:
		// No type constraint: TypeJSON accepts any JSON value --
		// an object, an array, a scalar -- and the renderer passes
		// it through untouched. Declaring "object" here made the
		// broker's schema validation refuse array answers the
		// recipe would have rendered fine (code review on #2646).
		return ""
	}
	return "string"
}

func decodeLiteral(p Placeholder, lit string) (any, bool) {
	var v any
	if json.Unmarshal([]byte(lit), &v) == nil {
		return v, true
	}
	if p.Type == TypeString {
		return strings.Trim(lit, "\"'`"), true
	}
	return nil, false
}

// The languages a recipe body can be written in, for literal
// rendering. langJS is the empty string because JavaScript is the
// historical default: a recipe with no language evidence renders
// exactly as it always has.
const (
	langJS     = ""
	langPython = "python"
	langShell  = "shell"
)

// language names the language this recipe's body runs in, from the
// same evidence the runner uses: a shebang first, then the file
// extension. Only the languages whose literal spellings differ from
// JavaScript's are named; everything else is langJS.
func (r Recipe) language() string {
	first := r.Source
	if i := strings.IndexByte(first, '\n'); i >= 0 {
		first = first[:i]
	}
	if strings.HasPrefix(first, "#!") {
		fields := strings.Fields(strings.TrimPrefix(first, "#!"))
		if len(fields) > 0 {
			interp := filepath.Base(fields[0])
			if interp == "env" && len(fields) > 1 {
				interp = filepath.Base(fields[1])
			}
			switch {
			case interp == "uv" || strings.HasPrefix(interp, "python"):
				return langPython
			case isShellName(interp):
				return langShell
			}
		}
	}
	switch strings.ToLower(filepath.Ext(r.Path)) {
	case ".py":
		return langPython
	case ".sh", ".bash", ".zsh", ".ksh":
		return langShell
	}
	return langJS
}

func isShellName(name string) bool {
	switch name {
	case "sh", "bash", "dash", "zsh", "ksh", "ash", "mksh":
		return true
	}
	return false
}

// boolLiteral spells a boolean in the body's language.
func boolLiteral(lang string, b bool) string {
	if lang == langPython {
		if b {
			return "True"
		}
		return "False"
	}
	return strconv.FormatBool(b)
}

// undefinedLiteral spells "no value" in the body's language: the
// value an omitted optional parameter renders as.
func undefinedLiteral(lang string) string {
	switch lang {
	case langPython:
		return "None"
	case langShell:
		return "''"
	}
	return "undefined"
}

// valueLiteral renders a value that reached the end of literalFor:
// a string, a JSON value, or anything else the placeholder's type
// did not specialise.
func valueLiteral(lang string, p Placeholder, v any) (string, error) {
	switch lang {
	case langPython:
		return pythonValue(v)
	case langShell:
		if s, ok := v.(string); ok {
			return shellQuote(s), nil
		}
		b, err := json.Marshal(v)
		if err != nil {
			return "", fmt.Errorf("%s cannot be encoded: %w", p.Name, err)
		}
		return shellQuote(string(b)), nil
	}
	b, err := json.Marshal(v)
	if err != nil {
		return "", fmt.Errorf("%s cannot be encoded: %w", p.Name, err)
	}
	return string(b), nil
}

// pythonValue renders v as a Python literal. JSON is almost Python;
// the differences are the three keywords and the container syntax,
// both handled by walking the value instead of patching the text.
func pythonValue(v any) (string, error) {
	switch t := v.(type) {
	case nil:
		return "None", nil
	case bool:
		return boolLiteral(langPython, t), nil
	case string:
		b, err := json.Marshal(t)
		if err != nil {
			return "", err
		}
		return string(b), nil
	case []any:
		parts := make([]string, len(t))
		for i, e := range t {
			s, err := pythonValue(e)
			if err != nil {
				return "", err
			}
			parts[i] = s
		}
		return "[" + strings.Join(parts, ", ") + "]", nil
	case map[string]any:
		keys := make([]string, 0, len(t))
		for k := range t {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		parts := make([]string, 0, len(t))
		for _, k := range keys {
			kb, err := json.Marshal(k)
			if err != nil {
				return "", err
			}
			s, err := pythonValue(t[k])
			if err != nil {
				return "", err
			}
			parts = append(parts, string(kb)+": "+s)
		}
		return "{" + strings.Join(parts, ", ") + "}", nil
	default:
		b, err := json.Marshal(v)
		if err != nil {
			return "", err
		}
		return string(b), nil
	}
}

// shellQuote renders s as one POSIX shell word: single-quoted, with
// embedded single quotes closed, escaped and reopened. Inside single
// quotes the shell performs no word splitting, no globbing and no
// command substitution, so any value stays exactly one data word.
func shellQuote(s string) string {
	return "'" + strings.ReplaceAll(s, "'", `'\''`) + "'"
}

// decodeLiteralFor is decodeLiteral in the body's language: a Python
// default may be spelled with Python's own keywords.
func decodeLiteralFor(lang string, p Placeholder, lit string) (any, bool) {
	if lang == langPython {
		switch strings.TrimSpace(lit) {
		case "True":
			return true, true
		case "False":
			return false, true
		case "None":
			return nil, true
		}
	}
	return decodeLiteral(p, lit)
}

// scanCodeFor is scanCode in the body's language.
func scanCodeFor(src string, lang string) []bool {
	if lang == langJS {
		return scanCode(src)
	}
	return scanHashCode(src, lang == langPython)
}

// scanHashCode is the code mask for the #-comment languages (Python
// and POSIX shell), where scanCode's JavaScript comment forms do not
// exist: a `#` comment runs to the end of the line, and placeholders
// inside it or inside a string are text, not holes. Python strings
// escape with backslashes and come in triple-quoted form; a shell
// single-quoted string has no escapes and may span lines.
func scanHashCode(src string, python bool) []bool {
	code := make([]bool, len(src))
	const (
		stCode = iota
		stComment
		stSingle
		stDouble
		stTripleSingle
		stTripleDouble
	)
	state := stCode
	for i := 0; i < len(src); i++ {
		c := src[i]
		switch state {
		case stCode:
			switch {
			case c == '#' && (python || i == 0 || strings.IndexByte(" \t\n;&|()", src[i-1]) >= 0):
				state = stComment
			case c == '\'' && python && i+2 < len(src) && src[i+1] == '\'' && src[i+2] == '\'':
				state = stTripleSingle
				i += 2
			case c == '"':
				if python && i+2 < len(src) && src[i+1] == '"' && src[i+2] == '"' {
					state = stTripleDouble
					i += 2
				} else {
					state = stDouble
				}
			case c == '\'':
				state = stSingle
			default:
				code[i] = true
			}
		case stComment:
			if c == '\n' {
				state = stCode
				code[i] = true
			}
		case stSingle:
			switch {
			case python && c == '\\':
				i++
			case c == '\'':
				state = stCode
			case c == '\n' && python:
				state = stCode
				code[i] = true
			}
		case stDouble:
			switch {
			case c == '\\':
				i++
			case c == '"':
				state = stCode
			case c == '\n' && python:
				state = stCode
				code[i] = true
			}
		case stTripleSingle, stTripleDouble:
			q := byte('\'')
			if state == stTripleDouble {
				q = '"'
			}
			if c == '\\' {
				i++
			} else if c == q && i+2 < len(src) && src[i+1] == q && src[i+2] == q {
				state = stCode
				i += 2
			}
		}
	}
	return code
}
