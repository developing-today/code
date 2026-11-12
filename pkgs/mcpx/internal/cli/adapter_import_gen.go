package cli

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/adapter"
)

// Declaration generation for `mcpx adapter import` (#1962 steps 2-5).
//
// The design is the issue's Approach C: a source ladder, best evidence
// first -- shell completions, then the man page, then --help -- emitting
// a file the user keeps, with provenance recorded on the declaration.
// Every parser here is deterministic: the same source text always
// produces the same declaration (GeneratedAt aside), no model is
// anywhere in the loop, and a source that yields no subcommands and no
// flags is REFUSED with a clear error rather than emitted as a partial
// declaration that looks complete.

// genFlag is one option extracted from a source.
type genFlag struct {
	long        string // "--output" (preferred form), or ""
	short       string // "-o", or ""
	arg         string // placeholder for the value it takes, "" = boolean
	description string
}

// genTool is one subcommand extracted from a source.
type genTool struct {
	name        string
	description string
	flags       []genFlag
}

// genParsed is what one source yielded.
type genParsed struct {
	description string
	tools       []genTool
	global      []genFlag
}

func (p *genParsed) usable() bool { return len(p.tools) > 0 || len(p.global) > 0 }

func (p *genParsed) addTool(name, description string) *genTool {
	name = normalizeGenToolName(name)
	for i := range p.tools {
		if p.tools[i].name == name {
			if p.tools[i].description == "" {
				p.tools[i].description = description
			}
			return &p.tools[i]
		}
	}
	p.tools = append(p.tools, genTool{name: name, description: description})
	return &p.tools[len(p.tools)-1]
}

func (p *genParsed) addGlobal(f genFlag) {
	p.global = appendFlagDedup(p.global, f)
}

func (t *genTool) addFlag(f genFlag) {
	t.flags = appendFlagDedup(t.flags, f)
}

// appendFlagDedup keeps the first flag of a given long (or short)
// name: a source that lists an option twice (a summary and a detail
// block) describes the same parameter once.
func appendFlagDedup(list []genFlag, f genFlag) []genFlag {
	key := f.long
	if key == "" {
		key = f.short
	}
	if key == "" {
		return list
	}
	for _, e := range list {
		k := e.long
		if k == "" {
			k = e.short
		}
		if k == key {
			return list
		}
	}
	return append(list, f)
}

func normalizeGenToolName(name string) string {
	name = strings.ToLower(strings.TrimSpace(name))
	name = strings.ReplaceAll(name, ".", "-")
	return name
}

var genCommandNameRe = regexp.MustCompile(`^[a-z][a-z0-9_-]*$`)

// param converts an extracted flag to a declaration parameter. The
// parameter name is the long flag with dashes as underscores (a JSON
// key a caller can actually type); a short-only flag keeps its letter.
func (f genFlag) param() adapter.Param {
	name := ""
	flag := f.long
	if f.long != "" {
		name = strings.ReplaceAll(strings.TrimPrefix(f.long, "--"), "-", "_")
	} else {
		name = strings.TrimPrefix(f.short, "-")
		flag = f.short
	}
	p := adapter.Param{Name: name, Flag: flag, Description: f.description}
	if f.arg == "" {
		p.Type = "boolean"
	}
	return p
}

var flagTokenRe = regexp.MustCompile(`^--?[A-Za-z][\w-]*$`)

// parseFlagLine extracts the flag a line leads with, if any. Lines
// that merely mention a flag mid-sentence are not declarations.
//
// The flag spec runs to the first run of two or more spaces (the
// column break every help/man formatter uses before the
// description), or to end of line: "-o <file>, --output <file>" is
// all spec, "-v, --verbose   Say more" is spec plus description.
// Inside the spec, comma-separated forms are one flag's spellings,
// each optionally followed by its value placeholder (<x>, [x],
// =<x>, or an UPPERCASE word).
func parseFlagLine(line string) (genFlag, bool) {
	rest := strings.TrimLeft(line, " \t")
	if !strings.HasPrefix(rest, "-") {
		return genFlag{}, false
	}
	spec, desc := rest, ""
	if idx := strings.IndexFunc(rest, func(r rune) bool { return r == '\t' }); idx >= 0 {
		spec, desc = rest[:idx], strings.TrimSpace(rest[idx:])
	} else if idx := strings.Index(rest, "  "); idx >= 0 {
		spec, desc = rest[:idx], strings.TrimSpace(rest[idx:])
	}
	var f genFlag
	for _, part := range strings.Split(spec, ",") {
		fields := strings.Fields(strings.TrimSpace(part))
		if len(fields) == 0 {
			continue
		}
		token := fields[0]
		arg := ""
		if name, value, ok := strings.Cut(token, "="); ok {
			token, arg = name, value
		}
		if !flagTokenRe.MatchString(token) {
			return genFlag{}, false
		}
		if strings.HasPrefix(token, "--") {
			f.long = token
		} else {
			f.short = token
		}
		if arg == "" && len(fields) > 1 && isPlaceholder(fields[1]) {
			arg = fields[1]
		}
		if arg != "" && f.arg == "" {
			f.arg = arg
		}
	}
	if f.long == "" && f.short == "" {
		return genFlag{}, false
	}
	if f.long == "--help" || f.long == "--version" || f.short == "-h" {
		// Meta flags every program has; declaring them teaches a
		// caller nothing about this program's surface.
		return genFlag{}, false
	}
	f.description = strings.Trim(desc, ":")
	return f, true
}

// isPlaceholder says whether a token reads as a flag's value
// placeholder rather than the start of a description.
func isPlaceholder(tok string) bool {
	if strings.HasPrefix(tok, "<") || strings.HasPrefix(tok, "[") {
		return true
	}
	// An all-caps word: FILE, N, PATH.
	return tok == strings.ToUpper(tok) && strings.IndexFunc(tok, func(r rune) bool {
		return r >= 'A' && r <= 'Z'
	}) >= 0
}

// commandEntry parses one commands-section line. An entry is a
// name alone, a name followed by positional placeholders
// (list [PATTERN...], up DEVICES...), or either followed by a
// column gap (two or more spaces, or a tab) and its description --
// the layout every help and man formatter uses for a list. A
// single-space line carrying no placeholders is prose that happens
// to sit in the section (a paragraph's tail line starts with a
// lowercase word too), and is not an entry.
func commandEntry(trimmed string) (name, desc string, ok bool) {
	// Tokenise, remembering each token's offset: the tokens after
	// the name are first the command's own positional syntax, then
	// its description column.
	type entryToken struct {
		text  string
		start int
		gap   string
	}
	var toks []entryToken
	for i := 0; i < len(trimmed); {
		gapStart := i
		for i < len(trimmed) && (trimmed[i] == ' ' || trimmed[i] == '\t') {
			i++
		}
		gap := trimmed[gapStart:i]
		if i >= len(trimmed) {
			break
		}
		start := i
		for i < len(trimmed) && trimmed[i] != ' ' && trimmed[i] != '\t' {
			i++
		}
		toks = append(toks, entryToken{text: trimmed[start:i], start: start, gap: gap})
	}
	if len(toks) == 0 {
		return "", "", false
	}
	name = stripManRef(toks[0].text)
	if !genCommandNameRe.MatchString(name) || name == "help" {
		return "", "", false
	}
	rest := toks[1:]
	// Consume the synopsis's positional placeholders: they are part
	// of the command's own syntax, not its description.
	consumed := 0
	for consumed < len(rest) && isPlaceholder(rest[consumed].text) {
		consumed++
	}
	rest = rest[consumed:]
	if len(rest) == 0 {
		// A name alone, or a name with only its positionals.
		return name, "", true
	}
	// What remains is the description. With no positionals consumed
	// it must start after a column gap: a single space means prose.
	// After consumed positionals the synopsis itself is the signal,
	// and the remainder is the description whatever its gap.
	if consumed == 0 && !strings.HasPrefix(rest[0].gap, "  ") && !strings.Contains(rest[0].gap, "\t") {
		return "", "", false
	}
	return name, trimmed[rest[0].start:], true
}

// stripManRef removes a trailing man-page section reference, the
// form a man COMMANDS list uses for its entries: git-add(1) is the
// add subcommand's page, named in full.
func stripManRef(name string) string {
	if i := strings.Index(name, "("); i > 0 && strings.HasSuffix(name, ")") &&
		strings.Trim(name[i:], "()0123456789") == "" {
		return name[:i]
	}
	return name
}

// sectionKind classifies a section header line in help/man text.
func sectionKind(header string) string {
	h := strings.ToLower(strings.TrimSpace(header))
	switch {
	case strings.Contains(h, "command"):
		return "commands"
	case strings.Contains(h, "option"), strings.Contains(h, "flag"):
		return "options"
	default:
		return "other"
	}
}

// parseHelpText parses --help output. Oracle: the text itself --
// every subcommand listed under a commands heading and every flag
// listed under an options heading (or leading a line anywhere)
// appears in the result exactly once, in source order.
func parseHelpText(text string) genParsed {
	var p genParsed
	section := ""
	inUsage := false // a usage block may wrap over several lines
	for _, line := range strings.Split(text, "\n") {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" {
			inUsage = false
			continue
		}
		if strings.HasSuffix(trimmed, ":") && !strings.HasPrefix(trimmed, "-") &&
			len(trimmed) < 80 && !strings.Contains(trimmed, "  ") {
			inUsage = false
			section = sectionKind(trimmed)
			continue
		}
		if strings.HasPrefix(strings.ToLower(trimmed), "usage") {
			inUsage = true
			continue
		}
		if inUsage {
			// A wrapped usage line is syntax, not a description.
			continue
		}
		indented := strings.HasPrefix(line, " ") || strings.HasPrefix(line, "\t")
		switch {
		case section == "commands" && indented:
			if name, desc, ok := commandEntry(trimmed); ok {
				p.addTool(name, desc)
			}
		case strings.HasPrefix(trimmed, "-"):
			if f, ok := parseFlagLine(line); ok {
				p.addGlobal(f)
			}
		case p.description == "" && section == "" &&
			!strings.HasPrefix(strings.ToLower(trimmed), "usage"):
			// A one-line summary before the usage block, when the
			// program offers one, is its own description of itself.
			if strings.Contains(trimmed, " ") {
				p.description = trimmed
			}
		}
	}
	return p
}

// parseManText parses man output (plain text, backspace overstrike
// already stripped, or mandoc markdown). Sections are ALL-CAPS (or
// ##) headers; OPTIONS entries lead with their flag and may carry
// their description on following, deeper-indented lines.
func parseManText(text string) genParsed {
	var p genParsed
	section := ""
	lastFlag := -1 // index into p.global, -1 = none
	lastTool := -1 // index into p.tools, -1 = none
	flush := func() { lastFlag = -1 }
	lines := strings.Split(text, "\n")
	indentOf := func(s string) int { return len(s) - len(strings.TrimLeft(s, " \t")) }
	for idx, raw := range lines {
		line := strings.ReplaceAll(raw, "`", "")
		trimmed := strings.TrimSpace(line)
		if trimmed == "" {
			flush()
			continue
		}
		if h, ok := manHeader(line); ok {
			section = h // raw lower-cased header; kind via sectionKind
			flush()
			continue
		}
		switch sectionKind(section) {
		case "options":
			if f, ok := parseFlagLine(line); ok {
				p.addGlobal(f)
				lastFlag = len(p.global) - 1
			} else if lastFlag >= 0 && (strings.HasPrefix(line, " ") || strings.HasPrefix(line, "\t")) {
				if p.global[lastFlag].description != "" {
					p.global[lastFlag].description += " "
				}
				p.global[lastFlag].description += trimmed
			}
		case "commands":
			// A man command entry is a short line (a name, maybe a
			// one-line gloss); the section's own prose paragraphs
			// are margin-filled lines far longer than any entry,
			// and a lowercase first word in one is not a command.
			if (strings.HasPrefix(line, " ") || strings.HasPrefix(line, "\t")) && len(trimmed) <= 64 {
				if name, desc, ok := commandEntry(trimmed); ok {
					alone := desc == "" && !strings.Contains(trimmed, " ")
					if alone {
						// A name alone is an entry only when its
						// description block follows, indented
						// deeper -- a lone word at a paragraph's
						// tail has no such block.
						deeper := false
						for _, next := range lines[idx+1:] {
							if strings.TrimSpace(next) == "" {
								continue
							}
							deeper = indentOf(next) > indentOf(line)
							break
						}
						if !deeper {
							flush()
							continue
						}
					}
					p.addTool(name, desc)
					lastTool = len(p.tools) - 1
				} else if lastTool >= 0 && p.tools[lastTool].description == "" {
					// A man command's description often sits on
					// the following line of its own.
					p.tools[lastTool].description = trimmed
				}
			}
			flush()
		default:
			// NAME holds "tool - what it is" in one line.
			if section == "name" {
				if _, desc, ok := strings.Cut(trimmed, " - "); ok && p.description == "" {
					p.description = strings.TrimSpace(desc)
				}
			}
			flush()
		}
	}
	return p
}

// manHeader recognises a man section header: an ALL-CAPS line at
// column zero, or a markdown ## header.
func manHeader(line string) (string, bool) {
	if strings.HasPrefix(line, "## ") {
		return strings.TrimSpace(strings.TrimPrefix(line, "## ")), true
	}
	if strings.HasPrefix(line, " ") || strings.HasPrefix(line, "\t") {
		return "", false
	}
	trimmed := strings.TrimSpace(line)
	if trimmed == "" || len(trimmed) > 40 {
		return "", false
	}
	upper := strings.ToUpper(trimmed)
	if upper == trimmed && strings.IndexFunc(trimmed, func(r rune) bool {
		return (r < 'A' || r > 'Z') && r != ' ' && r != '-'
	}) == -1 {
		return strings.ToLower(trimmed), true
	}
	return "", false
}

// fishTokens splits a fish line respecting single and double
// quotes, so -d 'Add file contents' stays one field.
func fishTokens(line string) []string {
	var out []string
	var cur strings.Builder
	var quote rune
	inTok := false
	for _, r := range line {
		switch {
		case quote != 0:
			if r == quote {
				quote = 0
			} else {
				cur.WriteRune(r)
			}
			inTok = true
		case r == '\'' || r == '"':
			quote = r
			inTok = true
		case r == ' ' || r == '\t':
			if inTok {
				out = append(out, cur.String())
				cur.Reset()
				inTok = false
			}
		default:
			cur.WriteRune(r)
			inTok = true
		}
	}
	if inTok {
		out = append(out, cur.String())
	}
	return out
}

// fishField extracts a fish complete field: -l value, --long value,
// or -l=value.
func fishField(line, short, long string) string {
	fields := fishTokens(line)
	for i, tok := range fields {
		for _, prefix := range []string{"-" + short, "--" + long} {
			if tok == prefix && i+1 < len(fields) {
				return fields[i+1]
			}
			if strings.HasPrefix(tok, prefix+"=") {
				return strings.TrimPrefix(tok, prefix+"=")
			}
		}
	}
	return ""
}

// parseCompletionText parses a shell completion script. Fish is a
// per-line declaration grammar; zsh _arguments specs are quoted
// 'flag[description]' strings and command lists are quoted
// 'name:description' pairs inside a commands array; bash word lists
// (-W "...") name the subcommands and its long-flag tokens name the
// options. All three reduce to the same genParsed.
func parseCompletionText(shell, text string) genParsed {
	var p genParsed
	switch shell {
	case "fish":
		for _, line := range strings.Split(text, "\n") {
			trimmed := strings.TrimSpace(line)
			if !strings.HasPrefix(trimmed, "complete ") {
				continue
			}
			long := fishField(trimmed, "l", "long-option")
			if long == "" {
				long = fishField(trimmed, "o", "old-option")
			}
			short := fishField(trimmed, "s", "short-option")
			desc := fishField(trimmed, "d", "description")
			cond := fishField(trimmed, "n", "condition")
			// The subcommand a conditional line belongs to.
			var tool *genTool
			if idx := strings.Index(cond, "__fish_seen_subcommand_from"); idx >= 0 {
				rest := strings.Fields(cond[idx+len("__fish_seen_subcommand_from"):])
				if len(rest) > 0 && genCommandNameRe.MatchString(normalizeGenToolName(rest[0])) {
					tool = p.addTool(rest[0], "")
				}
			}
			if long == "" && short == "" {
				// A bare -a word list under a use-subcommand
				// condition names the subcommands themselves.
				if strings.Contains(cond, "__fish_use_subcommand") {
					args := fishField(trimmed, "a", "arguments")
					for _, w := range strings.Fields(args) {
						if genCommandNameRe.MatchString(normalizeGenToolName(w)) {
							p.addTool(w, "")
						}
					}
				}
				continue
			}
			f := genFlag{description: desc}
			if long != "" {
				f.long = "--" + long
			}
			if short != "" {
				f.short = "-" + short
			}
			if strings.Contains(trimmed, " -r") || strings.Contains(trimmed, "--require-parameter") {
				f.arg = "<value>"
			}
			if f.long == "--help" {
				continue
			}
			if tool != nil {
				tool.addFlag(f)
			} else {
				p.addGlobal(f)
			}
		}
	case "zsh":
		inCommands := false
		for _, line := range strings.Split(text, "\n") {
			if strings.Contains(line, "commands") && strings.Contains(line, "=(") {
				inCommands = true
			}
			for _, seg := range quotedSegments(line) {
				if inCommands {
					if name, desc, ok := strings.Cut(seg, ":"); ok &&
						genCommandNameRe.MatchString(normalizeGenToolName(strings.TrimSpace(name))) {
						p.addTool(strings.TrimSpace(name), strings.TrimSpace(desc))
						continue
					}
				}
				if f, ok := parseZshFlagSpec(seg); ok {
					p.addGlobal(f)
				}
			}
			if strings.TrimSpace(line) == ")" {
				inCommands = false
			}
		}
	default: // bash
		wordListRe := regexp.MustCompile(`-W\s+"([^"]+)"|-W\s+'([^']+)'`)
		flagScanRe := regexp.MustCompile(`--[a-zA-Z][a-zA-Z0-9-]*`)
		for _, line := range strings.Split(text, "\n") {
			for _, m := range wordListRe.FindAllStringSubmatch(line, -1) {
				words := m[1]
				if words == "" {
					words = m[2]
				}
				for _, w := range strings.Fields(words) {
					if strings.HasPrefix(w, "-") || strings.Contains(w, "$") {
						continue
					}
					if genCommandNameRe.MatchString(normalizeGenToolName(w)) {
						p.addTool(w, "")
					}
				}
			}
			// Long flags appear in word lists and opts strings;
			// bash gives no descriptions or arity, so they are
			// recorded as booleans unless a later source rung
			// says otherwise.
			for _, tok := range flagScanRe.FindAllString(line, -1) {
				if tok != "--help" && tok != "--version" {
					// A bash flag's arity is unknowable from the
					// script alone; it is recorded value-taking so
					// callers can pass values, which is the common
					// case in opts word lists.
					p.addGlobal(genFlag{long: tok, arg: "<value>"})
				}
			}
		}
	}
	return p
}

// quotedSegments returns the single- and double-quoted strings on a
// line, in order.
func quotedSegments(line string) []string {
	var out []string
	for _, re := range []*regexp.Regexp{
		regexp.MustCompile(`'([^']*)'`),
		regexp.MustCompile(`"([^"]*)"`),
	} {
		for _, m := range re.FindAllStringSubmatch(line, -1) {
			out = append(out, m[1])
		}
	}
	return out
}

// parseZshFlagSpec parses one _arguments spec segment:
// '--verbose[be loud]', '--output=[file]:file:_files' or
// '(-q --quiet)' exclusion groups (refused: ambiguous which form
// the declaration should carry).
func parseZshFlagSpec(seg string) (genFlag, bool) {
	seg = strings.TrimSpace(strings.TrimPrefix(seg, "*"))
	if !strings.HasPrefix(seg, "-") {
		return genFlag{}, false
	}
	// The flag token runs to the first '[', '=' or ':'.
	end := strings.IndexAny(seg, "[=:")
	token := seg
	if end >= 0 {
		token = seg[:end]
	}
	if !regexp.MustCompile(`^--[A-Za-z][\w-]*$|^-[A-Za-z]$`).MatchString(token) {
		return genFlag{}, false
	}
	f := genFlag{}
	if strings.HasPrefix(token, "--") {
		f.long = token
	} else {
		f.short = token
	}
	if f.long == "--help" || f.long == "--version" {
		return genFlag{}, false
	}
	rest := seg[len(token):]
	if strings.HasPrefix(rest, "=[") {
		// '--output=[file]:...' -- the bracket is the value's
		// placeholder, not a description.
		f.arg = "<value>"
	} else if strings.HasPrefix(rest, "[") {
		if close := strings.Index(rest, "]"); close > 0 {
			f.description = rest[1:close]
			// '--json[format the output]:format:_fn' -- an
			// argument field after the description says the
			// option takes a value. Without it the flag is
			// boolean; dropping the field would expose a
			// value-taking option as a boolean.
			if suffix := rest[close+1:]; strings.HasPrefix(suffix, ":") &&
				strings.Trim(suffix, ":") != "" {
				f.arg = "<value>"
			}
		}
	} else if strings.HasPrefix(rest, "=") || strings.HasPrefix(rest, ":") {
		f.arg = "<value>"
	}
	return f, true
}

// buildGeneratedSpec turns one parsed source into a validated
// declaration with provenance. A source that yielded nothing usable
// is refused here: a partial declaration that looks complete is the
// failure mode #1962 exists to avoid.
func buildGeneratedSpec(name, command, source, sourceText string, parsed genParsed, binaryPath, binaryVersion string, now time.Time) (adapter.Spec, error) {
	if !parsed.usable() {
		return adapter.Spec{}, fmt.Errorf("no subcommands or flags could be parsed from %s for %q: refusing to emit a partial declaration; if %q has no declared surface worth exposing, use `mcpx adapter import --bare %s --description \"what it is for\"` instead",
			source, name, name, name)
	}
	spec := adapter.Spec{Name: name, Command: command}
	spec.Description = parsed.description
	if spec.Description == "" {
		spec.Description = fmt.Sprintf("%s, generated from its %s by mcpx adapter import", name, source)
	}
	globalParams := make([]adapter.Param, 0, len(parsed.global))
	for _, f := range parsed.global {
		globalParams = append(globalParams, f.param())
	}
	// Program-wide options stand before the subcommand when the
	// declaration is called, not after it (adapter.Param.Global).
	for i := range globalParams {
		globalParams[i].Global = true
	}
	if len(parsed.tools) == 0 {
		// A single-surface program: one tool named run, like the
		// bare passthrough's, but with the parsed flags declared.
		spec.Tools = []adapter.ToolSpec{{
			Name:        "run",
			Description: spec.Description,
			Params:      appendParams(globalParams, argsParam(name)),
		}}
	} else {
		seenTools := map[string]bool{}
		for _, t := range parsed.tools {
			// A man COMMANDS list names entries by their page:
			// git-add is the add subcommand of git. Strip the
			// adapter's own name as a prefix, drop an entry that
			// is the adapter itself, and dedupe what remains.
			tname := t.name
			if tname == name {
				continue
			}
			tname = strings.TrimPrefix(tname, name+"-")
			if seenTools[tname] {
				continue
			}
			seenTools[tname] = true
			params := make([]adapter.Param, 0, len(t.flags)+len(globalParams)+1)
			seen := map[string]bool{}
			add := func(ps []adapter.Param) {
				for _, prm := range ps {
					if !seen[prm.Name] {
						seen[prm.Name] = true
						params = append(params, prm)
					}
				}
			}
			toolParams := make([]adapter.Param, 0, len(t.flags))
			for _, f := range t.flags {
				toolParams = append(toolParams, f.param())
			}
			add(toolParams)   // the subcommand's own flags win
			add(globalParams) // then the program-wide ones
			params = appendParams(params, argsParam(tname))
			spec.Tools = append(spec.Tools, adapter.ToolSpec{
				Name:        tname,
				Description: t.description,
				Args:        []string{tname},
				Params:      params,
			})
		}
	}
	sum := sha256.Sum256([]byte(sourceText))
	spec.Provenance = &adapter.Provenance{
		Source:        source,
		BinaryPath:    binaryPath,
		BinaryVersion: binaryVersion,
		SourceSHA256:  hex.EncodeToString(sum[:]),
		GeneratedAt:   now.Format(time.RFC3339),
	}
	if err := spec.Validate(); err != nil {
		return adapter.Spec{}, fmt.Errorf("the declaration generated from %s for %q does not validate: %w", source, name, err)
	}
	return spec, nil
}

// argsParam is the trailing positional catch-all every generated
// tool carries: help and completion sources rarely enumerate a
// subcommand's positionals, and a declared array is honest about
// that where omitting it would make the tool unusable.
func argsParam(tool string) adapter.Param {
	return adapter.Param{
		Name:        "args",
		Type:        "array",
		Description: fmt.Sprintf("Additional arguments passed to %s, in order.", tool),
	}
}

func appendParams(base []adapter.Param, extra adapter.Param) []adapter.Param {
	for _, p := range base {
		if p.Name == extra.Name {
			extra.Name = "extra_args"
			break
		}
	}
	return append(append([]adapter.Param{}, base...), extra)
}

// --- probing: running and reading the binary's own sources ---

func runCapture(binary string, args ...string) (string, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, binary, args...)
	out, err := cmd.CombinedOutput()
	return string(out), err
}

// probeBinaryVersion resolves the binary and reads its version, the
// pair provenance records. Either may be empty without failing the
// import: a completion file can describe a program that is not here.
func probeBinaryVersion(binary string) (resolved, version string) {
	if p, err := exec.LookPath(binary); err == nil {
		resolved = p
	}
	if out, err := runCapture(binary, "--version"); err == nil {
		version = adapter.ExtractVersion(out)
	}
	return resolved, version
}

func probeHelp(binary string) (string, error) {
	var errs []string
	for _, arg := range []string{"--help", "-h", "help"} {
		out, err := runCapture(binary, arg)
		if strings.TrimSpace(out) != "" && (err == nil || arg == "help") {
			return out, nil
		}
		if err != nil {
			errs = append(errs, arg+": "+err.Error())
		}
	}
	return "", fmt.Errorf("no --help output from %q (%s)", binary, strings.Join(errs, "; "))
}

// stripOverstrike resolves the backspace sequences `man` uses for
// bold/underline in piped output, the way col -b does: a character
// written after a backspace replaces the one before it (N<BS>N is a
// bold N, _<BS>x an underlined x).
func stripOverstrike(s string) string {
	out := make([]rune, 0, len(s))
	overwrite := false
	for _, r := range s {
		if r == '\b' {
			overwrite = true
			continue
		}
		if overwrite && len(out) > 0 {
			out[len(out)-1] = r
			overwrite = false
			continue
		}
		out = append(out, r)
	}
	return string(out)
}

func probeMan(binary string) (string, error) {
	if _, err := exec.LookPath("man"); err != nil {
		return "", fmt.Errorf("no man program on PATH to read %q's page", binary)
	}
	base := filepath.Base(binary)
	out, err := runCapture("man", base)
	if err != nil || strings.TrimSpace(out) == "" {
		return "", fmt.Errorf("no man page for %q", base)
	}
	return stripOverstrike(out), nil
}

// systemCompletion reads a packager-shipped completion file for the
// binary from the standard directories -- the strongest source for
// old tools with no generator of their own (#1962 open question,
// answered yes: distributions already ship these).
func systemCompletion(base string) (shell, text string, ok bool) {
	candidates := []struct{ shell, path string }{
		{"fish", filepath.Join("/usr/share/fish/vendor_completions.d", base+".fish")},
		{"fish", filepath.Join("/usr/share/fish/completions", base+".fish")},
		{"zsh", filepath.Join("/usr/share/zsh/site-functions", "_"+base)},
		{"zsh", filepath.Join("/usr/share/zsh/vendor-completions", "_"+base)},
		{"bash", filepath.Join("/usr/share/bash-completion/completions", base)},
	}
	for _, c := range candidates {
		if b, err := os.ReadFile(c.path); err == nil && len(b) > 0 {
			return c.shell, string(b), true
		}
	}
	return "", "", false
}

// probeCompletion finds a completion source for the binary: a
// system file first, then the program's own generator
// (`<binary> completion <shell>`, the cobra/clap convention).
func probeCompletion(binary string) (shell, text string, err error) {
	base := filepath.Base(binary)
	if shell, text, ok := systemCompletion(base); ok {
		return shell, text, nil
	}
	for _, sh := range []string{"fish", "zsh", "bash"} {
		out, err := runCapture(binary, "completion", sh)
		if err == nil && strings.TrimSpace(out) != "" &&
			(strings.Contains(out, base) || strings.Contains(out, "complete")) {
			return sh, out, nil
		}
	}
	return "", "", fmt.Errorf("no completion source for %q: no system completion file and `%s completion <shell>` produced nothing", base, base)
}

// inferShell decides which completion grammar a file speaks, from
// its name first and its content second.
func inferShell(path, text string) string {
	base := filepath.Base(path)
	switch {
	case strings.HasSuffix(base, ".fish"):
		return "fish"
	case strings.HasPrefix(base, "_") || strings.HasSuffix(base, ".zsh"):
		return "zsh"
	case strings.HasSuffix(base, ".bash"):
		return "bash"
	}
	switch {
	case strings.Contains(text, "complete -c"):
		return "fish"
	case strings.Contains(text, "_arguments"):
		return "zsh"
	default:
		return "bash"
	}
}

// completionName derives the executable a completion file
// describes from the file's name: _git -> git, git.fish -> git,
// python3.12.fish -> python3.12. The executable keeps its real
// spelling -- it is what gets probed and run. The adapter
// namespace derived from it is normalized separately, by
// importNameAndCommand's rule (dots become dashes: a call names
// <name>.<tool> and splits at the first dot, so a dotted namespace
// would be unaddressable).
func completionName(path string) string {
	base := filepath.Base(path)
	base = strings.TrimPrefix(base, "_")
	if ext := filepath.Ext(base); ext != "" {
		base = strings.TrimSuffix(base, ext)
	}
	return base
}

// generateImport is adapterImport's non-bare path: walk the ladder
// for the requested source (or auto: completion, then man, then
// help) and build the declaration from the first usable rung.
func generateImport(opts *importOptions, now time.Time) (adapter.Spec, error) {
	if opts.completion != "" {
		// Every argument must be accounted for before any side
		// effect: --completion names the program itself, so a
		// positional binary alongside it would name a different
		// program than the file describes. Refuse it as the usage
		// error it is, before reading or probing anything.
		if len(opts.rest) != 0 {
			fmt.Fprintln(os.Stderr, importUsage)
			return adapter.Spec{}, fmt.Errorf("mcpx adapter import --completion takes no binary argument (got %s): the completion file names the program it describes; use --name to name a different one", strings.Join(opts.rest, " "))
		}
		data, err := os.ReadFile(opts.completion)
		if err != nil {
			return adapter.Spec{}, fmt.Errorf("reading completion file: %w", err)
		}
		shell := opts.shell
		if shell == "" {
			shell = inferShell(opts.completion, string(data))
		}
		// --name names the program the completion describes (its
		// binary and command), not just the adapter: a completion
		// file's own name is not always the program's (a fixture, a
		// download), and the name is the only place to say so. The
		// executable keeps its real spelling (python3.12); the
		// adapter namespace is that spelling with dots as dashes.
		binName := completionName(opts.completion)
		if opts.name != "" {
			binName = opts.name
		}
		name := strings.ReplaceAll(binName, ".", "-")
		resolved, version := probeBinaryVersion(binName)
		return buildGeneratedSpec(name, binName, "completion", string(data),
			parseCompletionText(shell, string(data)), resolved, version, now)
	}
	if len(opts.rest) != 1 {
		fmt.Fprintln(os.Stderr, importUsage)
		return adapter.Spec{}, fmt.Errorf("mcpx adapter import takes exactly one binary (or --completion <file>)")
	}
	binary := opts.rest[0]
	name, command := importNameAndCommand(binary)
	if opts.name != "" {
		name = opts.name
	}
	resolved, version := probeBinaryVersion(binary)
	if resolved == "" && !strings.ContainsRune(binary, '/') {
		return adapter.Spec{}, fmt.Errorf("cannot import %q: it is not on PATH; for a program that is not installed here, import its completion file with --completion instead", binary)
	}

	type rung struct {
		source string
		get    func() (string, genParsed, error)
	}
	completionRung := rung{"completion", func() (string, genParsed, error) {
		shell, text, err := probeCompletion(binary)
		if err != nil {
			return "", genParsed{}, err
		}
		return text, parseCompletionText(shell, text), nil
	}}
	manRung := rung{"man", func() (string, genParsed, error) {
		text, err := probeMan(binary)
		if err != nil {
			return "", genParsed{}, err
		}
		return text, parseManText(text), nil
	}}
	helpRung := rung{"help", func() (string, genParsed, error) {
		text, err := probeHelp(binary)
		if err != nil {
			return "", genParsed{}, err
		}
		return text, parseHelpText(text), nil
	}}

	var ladder []rung
	switch opts.source {
	case "completion":
		ladder = []rung{completionRung}
	case "man":
		ladder = []rung{manRung}
	case "help":
		ladder = []rung{helpRung}
	default: // auto
		ladder = []rung{completionRung, manRung, helpRung}
	}
	var tried []string
	for _, r := range ladder {
		text, parsed, err := r.get()
		if err != nil {
			tried = append(tried, r.source+": "+err.Error())
			continue
		}
		if !parsed.usable() {
			tried = append(tried, r.source+": nothing parseable in the source")
			continue
		}
		return buildGeneratedSpec(name, command, r.source, text, parsed, resolved, version, now)
	}
	return adapter.Spec{}, fmt.Errorf("mcpx adapter import could not generate a declaration for %q from any source (%s); if %q has no declared surface worth exposing, use --bare with a description instead",
		binary, strings.Join(tried, "; "), binary)
}
