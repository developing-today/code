// Import-graph stage of preflight: stage 1 (Graph) of #1915, report mode.
//
// mcpx used to learn about a broken import only when the runtime loaded the
// module -- after preflight, and on a cold cache after every server had
// already been started to read schemas. This stage computes the graph
// statically with `deno info --json` on the launcher (the launcher is what
// actually runs, which is already the rule runner.Run applies for
// typechecking), and reports every unresolved specifier in one pass, with
// the source span deno recorded for the edge that needed it.
//
// Report mode only: the deno info invocation is isolated so it cannot do
// the prepare stage's work by accident -- --no-lock and
// --node-modules-dir=none are appended after any forwarded resolution
// options, so a project configured with nodeModulesDir "auto" or a stale
// lockfile is neither installed into nor edited (Codex review, PR #2575).
// The prepare stage of #1915 stays out until the cache mechanism (#1908)
// and the resolved dependency set (#1913) exist to build on.
//
// The two edges a static graph cannot decide are computed dynamic imports:
// import("./" + name) and require(v). deno's graph simply omits them, so
// they are found by scanning the source of every local module in the graph
// and reported per script.importsOnDynamic -- never silently ignored.
package preflight

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"time"
)

// ImportsMode selects how far the import pipeline runs in this slice.
type ImportsMode string

const (
	ImportsOff    ImportsMode = "off"
	ImportsReport ImportsMode = "report"
)

// DynamicImportMode selects what a computed dynamic import does.
type DynamicImportMode string

const (
	DynamicWarn   DynamicImportMode = "warn"
	DynamicFail   DynamicImportMode = "fail"
	DynamicIgnore DynamicImportMode = "ignore"
)

// SourceSpan is one source location, 1-based, in the form editors print.
type SourceSpan struct {
	Path      string
	Line      int
	Character int
}

func (s SourceSpan) String() string {
	if s.Path == "" {
		return fmt.Sprintf("line %d, character %d", s.Line, s.Character)
	}
	return fmt.Sprintf("%s:%d:%d", s.Path, s.Line, s.Character)
}

// MissingSpecifier is one import edge whose target does not resolve.
type MissingSpecifier struct {
	// Specifier is the import as written in the source.
	Specifier string
	// Resolved is the absolute specifier deno resolved it to, when it got
	// that far.
	Resolved string
	// From is the module that imports it; empty when the entry itself is
	// the missing module.
	From string
	// Span locates the specifier in From, when deno recorded one.
	Span *SourceSpan
	// Err is deno's explanation, verbatim.
	Err string
}

// DynamicImport is one computed import()/require() a static graph cannot
// decide: the specifier is built at run time, so no graph can contain it.
type DynamicImport struct {
	// Script is the file the expression appears in.
	Script string
	// Line and Character locate the import/require keyword, 1-based.
	Line      int
	Character int
	// Expr is the argument source, trimmed.
	Expr string
	// Kind is "import" or "require".
	Kind string
}

// denoInfo mirrors the parts of `deno info --json` this stage reads.
type denoInfo struct {
	Roots   []string     `json:"roots"`
	Modules []denoModule `json:"modules"`
}

type denoModule struct {
	Kind         string           `json:"kind"`
	Specifier    string           `json:"specifier"`
	Local        string           `json:"local"`
	Error        string           `json:"error"`
	Dependencies []denoDependency `json:"dependencies"`
}

type denoDependency struct {
	Specifier string    `json:"specifier"`
	Code      *denoEdge `json:"code"`
	Type      *denoEdge `json:"type"`
}

type denoEdge struct {
	Specifier string   `json:"specifier"`
	Span      denoSpan `json:"span"`
	// Error is set instead of Specifier when deno itself could not
	// resolve this edge (an unmapped bare import, say): deno_graph's
	// serializer stores the failure on the dependency, not as a broken
	// module, and joining on the resolved specifier never sees it.
	Error string `json:"error"`
}

type denoSpan struct {
	Start denoPos `json:"start"`
	End   denoPos `json:"end"`
}

type denoPos struct {
	Line      int `json:"line"`
	Character int `json:"character"`
}

// Graph is the parsed result of one `deno info --json` run.
type Graph struct {
	// Missing is every unresolved edge, in source order by importer.
	Missing []MissingSpecifier
	// LocalFiles is every module deno read from disk; these are the files
	// scanned for computed dynamic imports.
	LocalFiles []string
}

// ParseGraph reads `deno info --json` output and derives the unresolved
// edges. A module deno could not load appears as an entry with an error and
// no kind; the edge that needed it is found by joining the dependencies of
// the modules that did load on their resolved specifier, which is what
// recovers the as-written specifier and its span for the report.
func ParseGraph(data []byte) (*Graph, error) {
	var info denoInfo
	if err := json.Unmarshal(data, &info); err != nil {
		return nil, fmt.Errorf("parse deno info output: %w", err)
	}
	g := &Graph{}
	broken := map[string]string{}
	for _, m := range info.Modules {
		if m.Error != "" {
			broken[m.Specifier] = m.Error
		}
		if m.Local != "" {
			g.LocalFiles = append(g.LocalFiles, m.Local)
		}
	}
	seen := map[string]bool{}
	add := func(ms MissingSpecifier) {
		key := ms.From + "\x00" + ms.Specifier + "\x00" + ms.Resolved
		if ms.Span != nil {
			key += fmt.Sprintf("\x00%d:%d", ms.Span.Line, ms.Span.Character)
		}
		if seen[key] {
			return
		}
		seen[key] = true
		g.Missing = append(g.Missing, ms)
	}
	for _, m := range info.Modules {
		if m.Error != "" {
			continue
		}
		for _, d := range m.Dependencies {
			for _, edge := range []*denoEdge{d.Code, d.Type} {
				if edge == nil {
					continue
				}
				if edge.Error != "" {
					// The failure lives on the edge itself; there is
					// no resolved specifier to join on.
					add(MissingSpecifier{
						Specifier: d.Specifier,
						From:      m.Specifier,
						Span: &SourceSpan{
							Path:      fileURLToPath(m.Specifier),
							Line:      edge.Span.Start.Line + 1,
							Character: edge.Span.Start.Character + 1,
						},
						Err: edge.Error,
					})
					continue
				}
				errText, isBroken := broken[edge.Specifier]
				if !isBroken {
					continue
				}
				span := &SourceSpan{
					Path:      fileURLToPath(m.Specifier),
					Line:      edge.Span.Start.Line + 1,
					Character: edge.Span.Start.Character + 1,
				}
				add(MissingSpecifier{
					Specifier: d.Specifier,
					Resolved:  edge.Specifier,
					From:      m.Specifier,
					Span:      span,
					Err:       errText,
				})
			}
		}
	}
	// A broken module no surviving edge points at -- the entry itself, or
	// the target of an edge kind this stage does not join -- is still
	// reported; dropping it would silently lose exactly the failures this
	// stage exists to surface.
	referenced := map[string]bool{}
	for _, ms := range g.Missing {
		referenced[ms.Resolved] = true
	}
	for _, m := range info.Modules {
		if m.Error != "" && !referenced[m.Specifier] {
			add(MissingSpecifier{Specifier: m.Specifier, Resolved: m.Specifier, Err: m.Error})
		}
	}
	sort.SliceStable(g.Missing, func(i, j int) bool {
		if g.Missing[i].From != g.Missing[j].From {
			return g.Missing[i].From < g.Missing[j].From
		}
		return g.Missing[i].Specifier < g.Missing[j].Specifier
	})
	sort.Strings(g.LocalFiles)
	return g, nil
}

func fileURLToPath(spec string) string {
	if rest, ok := strings.CutPrefix(spec, "file://"); ok {
		return rest
	}
	return spec
}

// ReportGraph folds a parsed graph and the dynamic imports found in its
// local files into a Report, one problem per missing specifier, all in one
// pass -- the TestEveryBadPathIsReportedInOnePass shape.
func ReportGraph(g *Graph, dynamics []DynamicImport, dynamicMode DynamicImportMode) *Report {
	r := &Report{}
	if g != nil {
		for _, ms := range g.Missing {
			what := fmt.Sprintf("unresolved import %q: %s", ms.Specifier, ms.Err)
			where := "script.imports"
			if ms.Span != nil {
				where = ms.Span.String()
			}
			r.Add(Problem{
				What:  what,
				Where: where,
				Fatal: true,
				Fix:   "fix the specifier, or declare the dependency so a later prepare stage can provide it; set script.imports off to skip this check",
			})
		}
	}
	r.mergeDynamics(dynamics, dynamicMode)
	return r
}

func (r *Report) mergeDynamics(dynamics []DynamicImport, mode DynamicImportMode) {
	if mode == DynamicIgnore {
		return
	}
	for _, d := range dynamics {
		loc := SourceSpan{Path: d.Script, Line: d.Line, Character: d.Character}
		r.Add(Problem{
			What:  fmt.Sprintf("computed %s(%s) cannot be resolved statically; mcpx cannot tell what it will load", d.Kind, d.Expr),
			Where: "script.imports in " + loc.String(),
			Fatal: mode == DynamicFail,
			Fix:   "declare the dependency it loads, or set script.importsOnDynamic ignore if the set is known another way",
		})
	}
}

// CheckImports computes the import graph of entry with deno and reports
// every unresolved specifier plus every computed dynamic import in the
// graph's local files. denoBin is the deno binary to use; the graph is a
// property of the source, not of the runtime that will execute it, so any
// deno serves. An empty denoBin is reported, not silently skipped: a check
// that did not run must not look like a check that passed.
// ResolutionArgs selects the arguments of a configured runtime that
// decide how specifiers resolve -- an import map can redirect a specifier
// to a different module, and a config file can carry the map, workspace
// and node-modules policy -- in both --flag value and --flag=value forms.
// Everything else a runtime is configured with (permission flags, --quiet,
// run-only switches) does not change the graph and is not forwarded: deno
// info rejects several of them, and a check that errors on its own flags
// would report scripts as broken that run fine.
func ResolutionArgs(args []string) []string {
	valueFlags := map[string]bool{
		"--import-map": true, "--config": true,
		"--node-modules-dir": true, "--lock": true,
	}
	switchFlags := map[string]bool{
		"--frozen": true, "--cached-only": true, "--no-remote": true,
	}
	var out []string
	for i := 0; i < len(args); i++ {
		a := args[i]
		if k, _, isEq := strings.Cut(a, "="); isEq {
			if valueFlags[k] || switchFlags[k] {
				out = append(out, a)
			}
			continue
		}
		if valueFlags[a] {
			out = append(out, a)
			if i+1 < len(args) {
				out = append(out, args[i+1])
				i++
			}
			continue
		}
		if switchFlags[a] {
			out = append(out, a)
		}
	}
	return out
}

// isolationArgs are appended LAST to every report-mode deno info call, so
// they win over any forwarded or project-configured value: report mode
// must not install dependencies or write a lockfile as a side effect of
// checking (deno info otherwise uses eager npm caching and writes a
// changed lockfile; nodeModulesDir "auto" would create node_modules).
var isolationArgs = []string{"--no-lock", "--node-modules-dir=none"}

func CheckImports(ctx context.Context, denoBin, entry string, mode ImportsMode, dynamicMode DynamicImportMode, timeout time.Duration) *Report {
	return CheckImportsResolved(ctx, denoBin, entry, mode, dynamicMode, timeout, nil, filepath.Dir(entry))
}

// CheckImportsResolved is CheckImports with the executing runtime's
// configured arguments: the resolution subset (ResolutionArgs) is passed
// to deno info, so the graph that is checked is the graph that will
// actually execute -- an import map that redirects ./dep.ts to a real
// module must not be reported as a missing import (Codex review, PR
// #2575).
//
// execDir is the directory the script will execute in (runner's
// Options.Dir). It matters because the runtime resolves a relative
// configuration path -- `--config ./deno.json` in the declared runtime
// arguments -- against its working directory, and deno info looks for
// configuration from there too. Computing the graph beside the entry
// file instead would check a different configuration from the one the
// script runs under, and report a valid import as missing (Codex
// review, PR #2575). An empty execDir inherits the caller's working
// directory, which is exactly what the execution does.
func CheckImportsResolved(ctx context.Context, denoBin, entry string, mode ImportsMode, dynamicMode DynamicImportMode, timeout time.Duration, runtimeArgs []string, execDir string) *Report {
	r := &Report{}
	if mode == ImportsOff || mode == "" {
		return r
	}
	if denoBin == "" {
		r.Add(Problem{
			What:  "the import graph was not checked: no deno binary is available to compute it",
			Where: "script.imports",
			Fix:   "install deno, or set script.imports off",
		})
		return r
	}
	cctx := ctx
	if timeout > 0 {
		var cancel context.CancelFunc
		cctx, cancel = context.WithTimeout(ctx, timeout)
		defer cancel()
	}
	argv := []string{"info", "--json"}
	argv = append(argv, ResolutionArgs(runtimeArgs)...)
	argv = append(argv, isolationArgs...)
	argv = append(argv, entry)
	cmd := exec.CommandContext(cctx, denoBin, argv...)
	// Execute in the directory the script will execute in, not beside
	// the entry file: relative --config paths and configuration discovery
	// resolve from the working directory (Codex review, PR #2575).
	cmd.Dir = execDir
	out, err := cmd.Output()
	if err != nil {
		if cctx.Err() != nil {
			r.Add(Problem{
				What:  "computing the import graph did not finish within " + timeout.String(),
				Where: "script.imports",
				Fix:   "raise the timeout, or set script.imports off",
			})
			return r
		}
		// Preserve the subprocess's own explanation: cmd.Output
		// captures it in ExitError.Stderr, and "exit status 1" alone
		// tells the caller nothing about why the graph failed
		// (Codex review, PR #2575). The type-check path already
		// surfaces its subprocess output the same way.
		what := "the import graph could not be computed: " + err.Error()
		var exitErr *exec.ExitError
		if errors.As(err, &exitErr) {
			if stderr := strings.TrimSpace(string(exitErr.Stderr)); stderr != "" {
				what += ": " + stderr
			}
		}
		r.Add(Problem{
			What:  what,
			Where: "script.imports",
			Fatal: true,
		})
		return r
	}
	g, perr := ParseGraph(out)
	if perr != nil {
		r.Add(Problem{What: perr.Error(), Where: "script.imports", Fatal: true})
		return r
	}
	var dynamics []DynamicImport
	for _, f := range g.LocalFiles {
		b, rerr := os.ReadFile(f)
		if rerr != nil {
			continue
		}
		dynamics = append(dynamics, ScanDynamicImports(f, string(b))...)
	}
	return ReportGraph(g, dynamics, dynamicMode)
}

// CheckImportsText is the degraded form for a script that arrives as text
// (remote exec): there is no file graph to compute, so only what is visible
// in the source is analysed -- its top-level bare specifiers, which are
// reported as unverifiable rather than missing (nothing here can tell
// whether the runtime would resolve them), and its computed dynamic
// imports, which are undecidable in any form.
func CheckImportsText(name, src string, dynamicMode DynamicImportMode) *Report {
	r := &Report{}
	bare, dynamics := ScanSourceImports(src)
	for _, spec := range bare {
		r.Add(Problem{
			What:  fmt.Sprintf("import %q is visible but cannot be verified: the script arrived as text, so there is no file graph to resolve it against", spec),
			Where: "script.imports",
			Fix:   "declare it with --package or in the dependency set if the runtime does not provide it",
		})
	}
	for i := range dynamics {
		dynamics[i].Script = name
	}
	r.mergeDynamics(dynamics, dynamicMode)
	return r
}

// ScanDynamicImports finds computed import()/require() calls in one source
// file: a literal argument is a static edge the graph already knows about;
// anything else -- a concatenation, a variable, a template with
// interpolation -- is undecidable and returned.
func ScanDynamicImports(name, src string) []DynamicImport {
	_, dynamics := scanSource(src)
	for i := range dynamics {
		dynamics[i].Script = name
	}
	return dynamics
}

// ScanSourceImports returns the visible top-level bare specifiers of a
// script (imports not relative to the script itself) and its computed
// dynamic imports. It scans rather than parses, in the diagnose package's
// shape: strings, comments and template literals are skipped rather than
// understood, so a specifier assembled dynamically is invisible here --
// which is the correct failure, and exactly what the dynamic-import report
// covers.
func ScanSourceImports(src string) (bare []string, dynamics []DynamicImport) {
	return scanSource(src)
}

func scanSource(src string) (bare []string, dynamics []DynamicImport) {
	seenBare := map[string]bool{}
	addBare := func(spec string) {
		if spec == "" || strings.HasPrefix(spec, ".") || strings.HasPrefix(spec, "/") ||
			strings.HasPrefix(spec, "file:") || strings.Contains(spec, "://") {
			return
		}
		if !seenBare[spec] {
			seenBare[spec] = true
			bare = append(bare, spec)
		}
	}
	lineStarts := []int{0}
	for i := 0; i < len(src); i++ {
		if src[i] == '\n' {
			lineStarts = append(lineStarts, i+1)
		}
	}
	pos := func(off int) (int, int) {
		lo, hi := 0, len(lineStarts)-1
		for lo < hi {
			mid := (lo + hi + 1) / 2
			if lineStarts[mid] <= off {
				lo = mid
			} else {
				hi = mid - 1
			}
		}
		return lo + 1, off - lineStarts[lo] + 1
	}
	prevWord := ""
	// prevValue records whether the previous significant token completes
	// a value (an identifier that is not an expression keyword, a number,
	// a string, a regex, a closing bracket). A '/' after a value divides;
	// anywhere else it can open a regex literal. Tracking only the last
	// word loses this: in (a + b) / (await import(name)) / 2 the ')' after
	// b completes a value, so both slashes divide and the import between
	// them is real code (Codex review, PR #2575).
	prevValue := false
	for i := 0; i < len(src); {
		c := src[i]
		switch {
		case c == '/' && i+1 < len(src) && (src[i+1] == '/' || src[i+1] == '*'):
			// Comments are recognised before regexes: at the start of
			// a line // opens a comment, never an empty regex, and
			// the comment's text is not code (Codex review, PR #2575).
			i = skipCommentIn(src, i)
			prevWord = ""
		case c == '/' && !prevValue:
			// A regex literal in a position where a value is expected:
			// its contents are not code, so an identifier+paren inside
			// one (/require(foo)/) is not a module load.
			if end, ok := scanRegexIn(src, i); ok {
				i = end
				prevWord = ""
				prevValue = true
				continue
			}
			i++
		case c == '\'' || c == '"' || c == '`':
			if c == '`' {
				// A template's literal text is not code, but its
				// ${...} interpolations are executable expressions:
				// scan each one (recursively, with positions remapped
				// into this file) or a computed import inside one is
				// silently missed (Codex review, PR #2575).
				end, spans := scanTemplateIn(src, i)
				for _, sp := range spans {
					_, inner := scanSource(src[sp[0]:sp[1]])
					baseLine, baseChar := pos(sp[0])
					for _, d := range inner {
						d.Line += baseLine - 1
						if d.Line == baseLine {
							d.Character += baseChar - 1
						}
						dynamics = append(dynamics, d)
					}
				}
				prevWord = ""
				prevValue = true
				i = end
				continue
			}
			end, content := readStringIn(src, i)
			if prevWord == "from" || prevWord == "import" {
				addBare(content)
			}
			prevWord = ""
			prevValue = true
			i = end
		case isIdentStartIn(c):
			word, next := readIdentIn(src, i)
			if (word == "import" || word == "require") && !precededByDot(src, i) {
				j := skipSpaceAndCommentsIn(src, next)
				if j < len(src) && src[j] == '(' && !isMethodDefinition(src, i, j, prevWord) {
					line, char := pos(i)
					dyn, lit, after := scanCallArg(src, j, word, line, char)
					if dyn != nil {
						dynamics = append(dynamics, *dyn)
					}
					addBare(lit)
					i = after
					prevWord = ""
					prevValue = true
					continue
				}
			}
			prevWord = word
			prevValue = precededByDot(src, i) || !expressionKeyword(word)
			i = next
		case c >= '0' && c <= '9':
			j := i
			for j < len(src) && (isIdentPartIn(src[j]) || src[j] == '.') {
				j++
			}
			prevWord = ""
			prevValue = true
			i = j
		default:
			if !isSpaceIn(c) {
				prevWord = ""
				prevValue = c == ')' || c == ']' || c == '}'
			}
			i++
		}
	}
	sort.Strings(bare)
	return bare, dynamics
}

// isMethodDefinition reports whether the parenthesised list at open is
// a method or function parameter list rather than a call's argument
// list. The body brace after the closing paren is necessary but not
// sufficient: automatic semicolon insertion makes a valid call statement
// followed by a block look identical -- `await import(name)\n{ ... }`
// parses as a call and a block, not a method (Codex review, PR #2575).
// So the name must also stand in declaration position: right after a
// member-list opening brace, a separator or a previous member, or after
// a declaration modifier. In valid code a call cannot be followed on the
// same line by a block, so a brace with no line terminator before it is
// always a body (`class Loader { require(value) { return value; } }` was
// reported as a computed require and rejected under
// script.importsOnDynamic=fail).
func isMethodDefinition(src string, ident, open int, prevWord string) bool {
	close := matchingParenIn(src, open)
	if close < 0 {
		return false
	}
	k := skipSpaceAndCommentsIn(src, close+1)
	if k >= len(src) || src[k] != '{' {
		return false
	}
	if !hasLineTerminator(src[close+1 : k]) {
		return true
	}
	return inDeclarationPosition(src, ident, prevWord)
}

// inDeclarationPosition reports whether the identifier at ident can begin
// a class or object-literal member: after the member-list brace, a comma
// or the end of the previous member, or after static/async/get/set (and
// after function for a declaration). After an expression keyword such as
// await, return or typeof the name is always an operand, so a brace that
// follows its argument list is a separate block.
func inDeclarationPosition(src string, ident int, prevWord string) bool {
	switch prevWord {
	case "static", "async", "get", "set", "function":
		return true
	}
	if prevWord != "" && expressionKeyword(prevWord) {
		return false
	}
	k := ident - 1
	for k >= 0 && isSpaceIn(src[k]) {
		k--
	}
	if k < 0 {
		return false
	}
	switch src[k] {
	case '{', ',', '}':
		return true
	}
	return false
}

// hasLineTerminator reports whether s contains a JavaScript line
// terminator, the condition under which automatic semicolon insertion
// can separate a call statement from the block that follows it.
func hasLineTerminator(s string) bool {
	return strings.ContainsAny(s, "\n\r")
}

// matchingParenIn returns the index of the ')' matching the '(' at
// open, skipping strings, templates and comments, or -1 when the list
// is unbalanced.
func matchingParenIn(src string, open int) int {
	depth := 0
	for i := open; i < len(src); {
		switch src[i] {
		case '\'', '"':
			i, _ = readStringIn(src, i)
		case '`':
			i, _ = scanTemplateIn(src, i)
		case '/':
			if i+1 < len(src) && (src[i+1] == '/' || src[i+1] == '*') {
				i = skipCommentIn(src, i)
			} else {
				i++
			}
		case '(':
			depth++
			i++
		case ')':
			depth--
			i++
			if depth == 0 {
				return i - 1
			}
		default:
			i++
		}
	}
	return -1
}

// precededByDot reports whether the identifier at off is a property
// access (obj.require, obj.import): the last non-space byte before it is
// a dot, so the name is a method, not the module loader.
func precededByDot(src string, off int) bool {
	k := off - 1
	for k >= 0 && isSpaceIn(src[k]) {
		k--
	}
	return k >= 0 && src[k] == '.'
}

// expressionKeyword reports whether an identifier is a keyword after
// which an expression -- and therefore possibly a regex literal --
// follows, rather than a value in its own right.
func expressionKeyword(word string) bool {
	switch word {
	case "return", "typeof", "case", "in", "of", "new", "delete",
		"void", "instanceof", "yield", "await", "throw", "else", "do":
		return true
	}
	return false
}

// scanRegexIn skips a regex literal starting at off (src[off] == '/'),
// honouring escapes and character classes. ok is false when no closing
// slash appears on the line — a division, not a literal.
func scanRegexIn(src string, off int) (int, bool) {
	inClass := false
	for j := off + 1; j < len(src); j++ {
		switch src[j] {
		case '\n':
			return 0, false
		case '\\':
			j++
		case '[':
			inClass = true
		case ']':
			inClass = false
		case '/':
			if !inClass {
				j++
				for j < len(src) && isIdentStartIn(src[j]) { // flags
					j++
				}
				return j, true
			}
		}
	}
	return 0, false
}

// scanCallArg inspects the argument of import(/require( starting at the
// opening parenthesis. A plain string literal is a static edge (returned as
// lit); anything else is a computed edge (returned as dyn).
// skipSpaceAndCommentsIn skips whitespace and comments: both can appear
// between a callee and its argument, and inside the argument list, where
// whitespace-only skipping misclassifies the call (Codex review, PR
// #2575: import(/* webpackIgnore: true */ "./x") is a literal import;
// import("./" /* c */ + name) is a computed one).
func skipSpaceAndCommentsIn(src string, i int) int {
	for i < len(src) {
		if isSpaceIn(src[i]) {
			i++
			continue
		}
		if src[i] == '/' && i+1 < len(src) && (src[i+1] == '/' || src[i+1] == '*') {
			i = skipCommentIn(src, i)
			continue
		}
		return i
	}
	return i
}

// scanTemplateIn returns the index just past the template literal
// starting at i (src[i] == '`') and the [start, end) spans of its ${...}
// interpolation expressions, whose contents are code even though the
// surrounding literal text is not.
func scanTemplateIn(src string, i int) (int, [][2]int) {
	var spans [][2]int
	for j := i + 1; j < len(src); {
		switch src[j] {
		case '\\':
			j += 2
		case '`':
			return j + 1, spans
		case '$':
			if j+1 >= len(src) || src[j+1] != '{' {
				j++
				continue
			}
			start := j + 2
			depth := 1
			k := start
			// prevValue mirrors scanSource: a '/' after a completed
			// value divides; anywhere else it can open a regex literal,
			// whose braces are not interpolation braces (Codex review,
			// PR #2575: `${/}/.test(s) ? await import(name) : ''}` lost
			// the import when the regex's } closed the span early).
			prevValue := false
			for k < len(src) && depth > 0 {
				switch {
				case src[k] == '\'' || src[k] == '"':
					k, _ = readStringIn(src, k)
					prevValue = true
				case src[k] == '`':
					k, _ = scanTemplateIn(src, k)
					prevValue = true
				case src[k] == '/' && k+1 < len(src) && (src[k+1] == '/' || src[k+1] == '*'):
					k = skipCommentIn(src, k)
				case src[k] == '/':
					if !prevValue {
						if end, ok := scanRegexIn(src, k); ok {
							k = end
							prevValue = true
							continue
						}
					}
					k++
					prevValue = false
				case src[k] == '{':
					depth++
					k++
					prevValue = false
				case src[k] == '}':
					depth--
					k++
					prevValue = true
				case isIdentStartIn(src[k]):
					word, next := readIdentIn(src, k)
					prevValue = !expressionKeyword(word)
					k = next
				case src[k] >= '0' && src[k] <= '9':
					for k < len(src) && (isIdentPartIn(src[k]) || src[k] == '.') {
						k++
					}
					prevValue = true
				default:
					if !isSpaceIn(src[k]) {
						prevValue = src[k] == ')' || src[k] == ']'
					}
					k++
				}
			}
			spans = append(spans, [2]int{start, k - 1})
			j = k
		default:
			j++
		}
	}
	return len(src), spans
}

func scanCallArg(src string, open int, kind string, line, char int) (dyn *DynamicImport, lit string, after int) {
	j := skipSpaceAndCommentsIn(src, open+1)
	if j >= len(src) {
		return nil, "", len(src)
	}
	switch src[j] {
	case '\'', '"', '`':
		end, content := readStringIn(src, j)
		// A string is a literal argument only when the token after it
		// terminates the argument -- ')' ends the call, ',' starts the
		// import-options argument. Anything else continues the
		// expression: '+' concatenates, '.concat(...)'/'.slice(...)'
		// compute on it, a template with interpolation is computed
		// wherever it stands (Codex review, PR #2575: import("./".concat(name))
		// was misread as a static edge and skipped).
		k := skipSpaceAndCommentsIn(src, end)
		computed := src[j] == '`' && strings.Contains(content, "${")
		if !computed && (k >= len(src) || (src[k] != ')' && src[k] != ',')) {
			computed = true
		}
		if !computed {
			return nil, content, end
		}
		fallthrough
	default:
		end := j
		depth := 0
		for end < len(src) {
			if src[end] == '(' {
				depth++
			}
			if src[end] == ')' {
				if depth == 0 {
					break
				}
				depth--
			}
			if src[end] == '\n' && depth == 0 {
				break
			}
			end++
		}
		expr := strings.TrimSpace(src[j:end])
		if len(expr) > 60 {
			expr = expr[:60] + "..."
		}
		return &DynamicImport{Line: line, Character: char, Expr: expr, Kind: kind}, "", end
	}
}

func isIdentStartIn(c byte) bool {
	return c == '_' || c == '$' || (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')
}

func isIdentPartIn(c byte) bool {
	return isIdentStartIn(c) || (c >= '0' && c <= '9')
}

func isSpaceIn(c byte) bool { return c == ' ' || c == '\t' || c == '\r' || c == '\n' }

func skipSpaceIn(src string, i int) int {
	for i < len(src) && isSpaceIn(src[i]) {
		i++
	}
	return i
}

func readIdentIn(src string, i int) (string, int) {
	j := i
	for j < len(src) && isIdentPartIn(src[j]) {
		j++
	}
	return src[i:j], j
}

func skipCommentIn(src string, i int) int {
	if i+1 < len(src) && src[i+1] == '/' {
		for i < len(src) && src[i] != '\n' {
			i++
		}
		return i
	}
	end := strings.Index(src[i+2:], "*/")
	if end < 0 {
		return len(src)
	}
	return i + 2 + end + 2
}

// readStringIn returns the index just past the literal starting at i and
// its raw content (escapes left as written; only the shape matters here).
func readStringIn(src string, i int) (int, string) {
	quote := src[i]
	j := i + 1
	for j < len(src) {
		if src[j] == '\\' {
			j += 2
			continue
		}
		if src[j] == quote {
			return j + 1, src[i+1 : j]
		}
		if src[j] == '\n' && quote != '`' {
			return j, src[i+1 : j]
		}
		j++
	}
	return len(src), src[i+1:]
}
