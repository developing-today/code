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
// Report mode only: nothing is installed, nothing is written into any
// project directory, and no cache entry is created. The prepare stage of
// #1915 stays out until the cache mechanism (#1908) and the resolved
// dependency set (#1913) exist to build on.
//
// The two edges a static graph cannot decide are computed dynamic imports:
// import("./" + name) and require(v). deno's graph simply omits them, so
// they are found by scanning the source of every local module in the graph
// and reported per script.importsOnDynamic -- never silently ignored.
package preflight

import (
	"context"
	"encoding/json"
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
func CheckImports(ctx context.Context, denoBin, entry string, mode ImportsMode, dynamicMode DynamicImportMode, timeout time.Duration) *Report {
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
	cmd := exec.CommandContext(cctx, denoBin, "info", "--json", entry)
	cmd.Dir = filepath.Dir(entry)
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
		r.Add(Problem{
			What:  "the import graph could not be computed: " + err.Error(),
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
	for i := 0; i < len(src); {
		c := src[i]
		switch {
		case c == '/' && i+1 < len(src) && (src[i+1] == '/' || src[i+1] == '*'):
			i = skipCommentIn(src, i)
			prevWord = ""
		case c == '\'' || c == '"' || c == '`':
			end, content := readStringIn(src, i)
			if prevWord == "from" || prevWord == "import" {
				addBare(content)
			}
			prevWord = ""
			i = end
		case isIdentStartIn(c):
			word, next := readIdentIn(src, i)
			if word == "import" || word == "require" {
				j := skipSpaceIn(src, next)
				if j < len(src) && src[j] == '(' {
					line, char := pos(i)
					dyn, lit, after := scanCallArg(src, j, word, line, char)
					if dyn != nil {
						dynamics = append(dynamics, *dyn)
					}
					addBare(lit)
					i = after
					prevWord = ""
					continue
				}
			}
			prevWord = word
			i = next
		default:
			if !isSpaceIn(c) {
				prevWord = ""
			}
			i++
		}
	}
	sort.Strings(bare)
	return bare, dynamics
}

// scanCallArg inspects the argument of import(/require( starting at the
// opening parenthesis. A plain string literal is a static edge (returned as
// lit); anything else is a computed edge (returned as dyn).
func scanCallArg(src string, open int, kind string, line, char int) (dyn *DynamicImport, lit string, after int) {
	j := skipSpaceIn(src, open+1)
	if j >= len(src) {
		return nil, "", len(src)
	}
	switch src[j] {
	case '\'', '"', '`':
		end, content := readStringIn(src, j)
		// A literal followed by + is a concatenation -- computed, not a
		// static edge -- and a template with interpolation is computed
		// wherever it stands.
		k := skipSpaceIn(src, end)
		computed := src[j] == '`' && strings.Contains(content, "${")
		if k < len(src) && src[k] == '+' {
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
