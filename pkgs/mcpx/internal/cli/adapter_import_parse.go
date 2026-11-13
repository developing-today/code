package cli

import (
	"context"
	"os"
	"os/exec"
	"regexp"
	"sort"
	"strings"
	"time"
)

// The source parsers for adapter import (#1962). Each is a pure function
// from text to importShape, so the corpus of real-world sources is a set
// of fixtures and the oracle for every test is a hand-written shape.

// ---- fish -------------------------------------------------------------
//
// fish completions are the most declarative source that exists:
//   complete -c git -n '__fish_seen_subcommand_from add' -s a -l all -d '...'
// Each line is one flag for one context; the -n condition names the
// subcommand it belongs to (or, via __fish_use_subcommand and friends,
// that it is a subcommand name itself or a top-level flag).

// fishTokens splits a fish line the way fish would for the simple shapes
// completion files use: single and double quotes group, backslash
// escapes inside double quotes and outside quotes.
func fishTokens(line string) []string {
	var out []string
	var cur strings.Builder
	quote := byte(0)
	flush := func() {
		if cur.Len() > 0 {
			out = append(out, cur.String())
			cur.Reset()
		}
	}
	for i := 0; i < len(line); i++ {
		c := line[i]
		if quote != 0 {
			if c == quote {
				quote = 0
			} else if c == '\\' && quote == '"' && i+1 < len(line) {
				i++
				cur.WriteByte(line[i])
			} else {
				cur.WriteByte(c)
			}
			continue
		}
		switch c {
		case '\'', '"':
			quote = c
		case '\\':
			if i+1 < len(line) {
				i++
				cur.WriteByte(line[i])
			}
		case ' ', '\t':
			flush()
		default:
			cur.WriteByte(c)
		}
	}
	flush()
	return out
}

var fishSeenSubRE = regexp.MustCompile(`__fish_seen_subcommand_from\s+([^;'"]+)`)

// fishSubcommandsOf reads the commands a -n condition scopes a flag to:
// every word after __fish_seen_subcommand_from. A condition may list
// several spellings of one command (mcpx's own generated fish
// completions join a command's aliases exactly this way), and the flag
// belongs to each of them — taking only the first word would strip the
// flag from every other spelling's tool.
func fishSubcommandsOf(cond string) []string {
	m := fishSeenSubRE.FindStringSubmatch(cond)
	if m == nil {
		return nil
	}
	return strings.Fields(m[1])
}

func parseFishCompletion(text, name string) *importShape {
	shape := &importShape{}
	bySub := map[string]*importSub{}
	subOrder := []string{}
	globalSeen := map[string]bool{}
	subSeen := map[string]map[string]bool{}
	getSub := func(sub string) *importSub {
		if bySub[sub] == nil {
			bySub[sub] = &importSub{Name: sub}
			subSeen[sub] = map[string]bool{}
			subOrder = append(subOrder, sub)
		}
		return bySub[sub]
	}
	addFlag := func(sub string, f importFlag) {
		key := f.flag()
		if sub == "" {
			if globalSeen[key] {
				return
			}
			globalSeen[key] = true
			shape.Global = append(shape.Global, f)
			return
		}
		s := getSub(sub)
		if subSeen[sub][key] {
			return
		}
		subSeen[sub][key] = true
		s.Flags = append(s.Flags, f)
	}
	valued := map[string]bool{
		"-c": true, "--command": true,
		"-s": true, "--short-option": true,
		"-l": true, "--long-option": true,
		"-o": true, "--old-option": true,
		"-d": true, "--description": true,
		"-n": true, "--condition": true,
		"-a": true, "--arguments": true,
		"-p": true, "--path": true,
		"-w": true, "--wraps": true,
		"-C": true,
	}
	for _, line := range strings.Split(text, "\n") {
		line = strings.TrimSpace(line)
		if !strings.HasPrefix(line, "complete ") && line != "complete" {
			continue
		}
		toks := fishTokens(line)
		if len(toks) < 2 {
			continue
		}
		var cmd, desc, args string
		var subs []string
		var flag importFlag
		isCompleteForSelf := false
		for i := 1; i < len(toks); i++ {
			t := toks[i]
			if valued[t] && i+1 < len(toks) {
				v := toks[i+1]
				i++
				switch t {
				case "-c", "--command":
					cmd = v
				case "-s", "--short-option":
					flag.Short = v
				case "-l", "--long-option":
					flag.Long = v
				case "-o", "--old-option":
					if flag.Long == "" {
						flag.Long = v
					}
				case "-d", "--description":
					desc = v
				case "-n", "--condition":
					subs = fishSubcommandsOf(v)
					isCompleteForSelf = isCompleteForSelf || strings.Contains(v, "__fish_use_subcommand")
				case "-a", "--arguments":
					args = v
				}
				continue
			}
			if t == "-r" || t == "--require-parameter" {
				flag.TakesValue = true
			}
		}
		if cmd != "" && cmd != name {
			continue // a file completing several programs; not ours
		}
		if args != "" && flag.Short == "" && flag.Long == "" {
			// An -a line with no flag declares subcommand names when it
			// is the use-subcommand context, or flag values otherwise;
			// only the former is structure mcpx can use.
			if isCompleteForSelf {
				for _, w := range strings.Fields(args) {
					if w == "help" {
						continue
					}
					getSub(w)
				}
			}
			continue
		}
		if flag.Short == "" && flag.Long == "" {
			continue
		}
		flag.Desc = desc
		if len(subs) == 0 {
			addFlag("", flag)
		}
		for _, sub := range subs {
			addFlag(sub, flag)
		}
	}
	sort.Strings(subOrder)
	for _, sub := range subOrder {
		shape.Subs = append(shape.Subs, *bySub[sub])
	}
	return shape
}

// ---- zsh --------------------------------------------------------------
//
// zsh completions are a real grammar (_arguments specs), and the
// hand-written ones vary too much to parse whole. What is regular is
// the flag spec itself: '--flag[description]', '--flag=[description]'
// (the = means it takes a value), '-f[description]', and ':...'
// continuations marking a value. Scan for those, in order.

var zshFlagRE = regexp.MustCompile(`(--[a-zA-Z0-9][a-zA-Z0-9_-]*)(=?)\[([^\]]*)\](:?)|-([a-zA-Z])(=?)\[([^\]]*)\](:?)`)

func parseZshCompletion(text, name string) *importShape {
	shape := &importShape{}
	seen := map[string]bool{}
	for _, m := range zshFlagRE.FindAllStringSubmatch(text, -1) {
		var f importFlag
		if m[1] != "" {
			f = importFlag{Long: strings.TrimPrefix(m[1], "--"), Desc: m[3], TakesValue: m[2] == "=" || m[4] == ":"}
		} else {
			f = importFlag{Short: m[5], Desc: m[7], TakesValue: m[6] == "=" || m[8] == ":"}
		}
		if seen[f.flag()] {
			continue
		}
		seen[f.flag()] = true
		shape.Global = append(shape.Global, f)
	}
	return shape
}

// ---- bash -------------------------------------------------------------
//
// bash completions are the least regular source: what is extractable
// deterministically is the word lists packagers and generators left in
// opts=/commands= assignments. Cobra- and clap-generated bash scripts
// delegate to the binary's __complete and carry no lists at all; those
// parse empty and the ladder moves on, which is the honest outcome.
// Bash word lists do not say which flags take values, so every flag is
// declared as a string parameter and import says so in a note on the
// spec (see buildGeneratedImport's caller): wrong for boolean flags,
// visible in the file, and fixable by editing the declaration.
// A list means what its enclosing handler says it means: completions
// name the handler for a nested command after the binary (_git_bisect
// completes "git bisect ..."), so a nested handler's subcommand list
// is emitted with its full command path, and a list whose enclosing
// function establishes no command context is skipped — promoting it
// would invent top-level commands the binary does not have (#9560).

var bashListRE = regexp.MustCompile(`(?m)(opts|options|commands|subcommands|cmds)\s*=\s*"([^"]*)"|(opts|options|commands|subcommands|cmds)\s*=\s*'([^']*)'`)

var bashFuncHeaderRE = regexp.MustCompile(`(?m)^[ \t]*(?:function[ \t]+)?([A-Za-z_][A-Za-z0-9_-]*)[ \t]*\([ \t]*\)`)

// bashFuncSpan is one function definition's byte span in the source.
type bashFuncSpan struct {
	name       string
	start, end int // end is just past the closing brace, or len(text)
}

// bashFuncSpans locates the function definitions in a bash completion
// file. A list assignment only means what its enclosing function says
// it means, so the parser needs the spans before it reads any list.
func bashFuncSpans(text string) []bashFuncSpan {
	var spans []bashFuncSpan
	for _, loc := range bashFuncHeaderRE.FindAllStringSubmatchIndex(text, -1) {
		span := bashFuncSpan{name: text[loc[2]:loc[3]], start: loc[0], end: len(text)}
		if brace := strings.Index(text[loc[1]:], "{"); brace >= 0 {
			span.end = bashFuncEnd(text, loc[1]+brace)
		}
		spans = append(spans, span)
	}
	return spans
}

// bashFuncEnd returns the offset just past the brace that closes the
// function whose opening brace is at open. Quoted strings and comments
// are skipped so braces inside them do not count; ${...} expansions
// outside quotes are balanced, so counting their braces is safe.
func bashFuncEnd(text string, open int) int {
	depth := 0
	for i := open; i < len(text); i++ {
		switch text[i] {
		case '\\':
			i++
		case '\'':
			for i++; i < len(text) && text[i] != '\''; i++ {
			}
		case '"':
			for i++; i < len(text); i++ {
				if text[i] == '\\' {
					i++
				} else if text[i] == '"' {
					break
				}
			}
		case '#':
			if i == 0 || strings.ContainsRune(" \t\n;|&(", rune(text[i-1])) {
				for i < len(text) && text[i] != '\n' {
					i++
				}
			}
		case '{':
			depth++
		case '}':
			depth--
			if depth == 0 {
				return i + 1
			}
		}
	}
	return len(text)
}

// bashHandlerPrefix maps an enclosing completion function to the
// command path its lists belong to, relative to the binary. Bash
// completion convention names the handler for a command path after
// the binary: _git completes git itself, _git_bisect completes
// "git bisect ...". A function that does not follow the convention
// (an internal helper like __git_ps1, or an unrelated function)
// establishes no context: ok is false and its lists must be skipped
// rather than promoted to top-level tools the binary does not have.
func bashHandlerPrefix(fn, name string) (prefix []string, ok bool) {
	base := strings.NewReplacer("-", "_", ".", "_").Replace(name)
	switch {
	case fn == base || fn == "_"+base:
		return nil, true
	case strings.HasPrefix(fn, "_"+base+"_"):
		rest := strings.TrimPrefix(fn, "_"+base+"_")
		return strings.Split(rest, "_"), true
	default:
		return nil, false
	}
}

func parseBashCompletion(text, name string) *importShape {
	shape := &importShape{}
	spans := bashFuncSpans(text)
	// contextAt resolves the command path a byte offset's lists belong
	// to: the innermost enclosing function decides. Outside any
	// function the lists are the file's own top-level declarations.
	contextAt := func(pos int) ([]string, bool) {
		inner := -1
		for i, sp := range spans {
			if sp.start <= pos && pos < sp.end && (inner < 0 || spans[inner].start <= sp.start) {
				inner = i
			}
		}
		if inner < 0 {
			return nil, true
		}
		return bashHandlerPrefix(spans[inner].name, name)
	}
	seenFlag := map[string]bool{}
	seenSub := map[string]bool{}
	subIndex := map[string]int{}
	subFlagSeen := map[string]map[string]bool{}
	// getSub returns the index of the sub for path, creating it if new.
	// An index, not a pointer: appending to shape.Subs may reallocate
	// the backing array, which would strand a stored pointer.
	getSub := func(path []string) int {
		joined := strings.Join(path, "_")
		if idx, ok := subIndex[joined]; ok {
			return idx
		}
		sub := importSub{Name: joined}
		if len(path) > 1 {
			sub.Path = append([]string(nil), path...)
		}
		shape.Subs = append(shape.Subs, sub)
		subIndex[joined] = len(shape.Subs) - 1
		subFlagSeen[joined] = map[string]bool{}
		seenSub[joined] = true
		return len(shape.Subs) - 1
	}
	for _, loc := range bashListRE.FindAllStringSubmatchIndex(text, -1) {
		kind, list := "", ""
		if loc[2] >= 0 {
			kind, list = text[loc[2]:loc[3]], text[loc[4]:loc[5]]
		} else {
			kind, list = text[loc[6]:loc[7]], text[loc[8]:loc[9]]
		}
		prefix, known := contextAt(loc[0])
		if !known {
			continue // no establishable context: never invent top-level tools
		}
		for _, w := range strings.Fields(list) {
			switch {
			case strings.HasPrefix(w, "--") && len(w) > 2:
				long := strings.TrimPrefix(w, "--")
				long = strings.TrimSuffix(long, "=")
				f := importFlag{Long: long, TakesValue: true}
				if len(prefix) == 0 {
					if !seenFlag[f.flag()] {
						seenFlag[f.flag()] = true
						shape.Global = append(shape.Global, f)
					}
				} else if idx := getSub(prefix); !subFlagSeen[shape.Subs[idx].Name][f.flag()] {
					subFlagSeen[shape.Subs[idx].Name][f.flag()] = true
					shape.Subs[idx].Flags = append(shape.Subs[idx].Flags, f)
				}
			case (kind == "commands" || kind == "subcommands" || kind == "cmds") && !strings.HasPrefix(w, "-"):
				if w == "help" {
					continue
				}
				path := append(append([]string(nil), prefix...), w)
				if !seenSub[strings.Join(path, "_")] {
					getSub(path)
				}
			}
		}
	}
	sort.Slice(shape.Subs, func(i, j int) bool { return shape.Subs[i].Name < shape.Subs[j].Name })
	return shape
}

// ---- man --------------------------------------------------------------
//
// man pages are troff, but import does not need troff: `man -P cat`
// renders the page, and the OPTIONS section of the rendering is a
// fixed shape -- a flag spec at one indent, its description deeper,
// until the next spec or section. NAME gives the one-line description.

func manShape(binary string) (*importShape, error) {
	if _, err := exec.LookPath("man"); err != nil {
		return nil, err
	}
	out, err := runCaptureEnv(map[string]string{"MANWIDTH": "100", "TERM": "dumb"}, "man", "-P", "cat", binary)
	if err != nil {
		return nil, err
	}
	return parseManText(out), nil
}

// runCaptureEnv is runCapture with extra environment.
func runCaptureEnv(env map[string]string, binary string, args ...string) (string, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, binary, args...)
	cmd.Env = os.Environ()
	for k, v := range env {
		cmd.Env = append(cmd.Env, k+"="+v)
	}
	var out strings.Builder
	cmd.Stdout = &out
	cmd.Stderr = &out
	err := cmd.Run()
	if out.Len() > 0 {
		return out.String(), nil
	}
	return "", err
}

// stripOverstrike removes the backspace overstrikes `man` output uses
// for bold and underline, leaving plain text.
func stripOverstrike(s string) string {
	var b strings.Builder
	for _, r := range s {
		if r == '\b' {
			str := b.String()
			if len(str) > 0 {
				b.Reset()
				b.WriteString(str[:len(str)-1])
			}
			continue
		}
		b.WriteRune(r)
	}
	return b.String()
}

func parseManText(text string) *importShape {
	text = stripOverstrike(text)
	shape := &importShape{}
	section := ""
	seen := map[string]bool{}
	entryIndent := 0
	var last *importFlag
	flushDesc := func() { last = nil }
	for _, line := range strings.Split(text, "\n") {
		trimmed := strings.TrimSpace(line)
		if trimmed != "" && trimmed == strings.ToUpper(trimmed) && len(trimmed) > 2 && !strings.HasPrefix(line, " ") && !strings.HasPrefix(line, "\t") {
			section = trimmed
			entryIndent = 0
			flushDesc()
			continue
		}
		switch {
		case strings.HasPrefix(section, "NAME"):
			if i := strings.Index(trimmed, "—"); i > 0 {
				shape.Description = strings.TrimSpace(trimmed[i+len("—"):])
			} else if i := strings.Index(trimmed, "- "); i > 0 && shape.Description == "" {
				shape.Description = strings.TrimSpace(trimmed[i+2:])
			}
		case strings.HasPrefix(section, "OPTIONS"):
			// Entry specs sit at the section's entry indent and their
			// descriptions deeper; a wrapped description line that
			// happens to start with a dash is not a spec. The first
			// spec line sets the indent.
			indent := len(line) - len(strings.TrimLeft(line, " "))
			if strings.HasPrefix(trimmed, "-") && (entryIndent == 0 || indent <= entryIndent) {
				if entryIndent == 0 {
					entryIndent = indent
				}
				flags := parseFlagSpec(trimmed)
				last = nil
				for _, f := range flags {
					if seen[f.flag()] {
						continue
					}
					seen[f.flag()] = true
					shape.Global = append(shape.Global, f)
					last = &shape.Global[len(shape.Global)-1]
				}
			} else if last != nil && trimmed != "" {
				if last.Desc != "" {
					last.Desc += " "
				}
				last.Desc += trimmed
			}
		}
	}
	return shape
}

// ---- --help -----------------------------------------------------------
//
// #114's plan, as the fallback it should be: usage line, a commands
// section, a flags/options section. Cobra, clap and argparse all emit
// help in this shape, which is the only reason parsing it is sound.

func helpShape(binary string) (*importShape, error) {
	out, err := runCapture(binary, "--help")
	if err != nil {
		return nil, err
	}
	shape := parseHelpText(out)
	// One level of recursion: a subcommand's own --help carries its
	// flags, and without them the generated tools would be names only.
	for i, sub := range shape.Subs {
		subOut, serr := runCapture(binary, sub.Name, "--help")
		if serr != nil {
			continue
		}
		subShape := parseHelpText(subOut)
		shape.Subs[i].Flags = subShape.Global
		shape.Subs[i].Positionals = subShape.Positionals
		if shape.Subs[i].Desc == "" {
			shape.Subs[i].Desc = firstSentence(subShape.Description)
		}
	}
	return shape, nil
}

func firstSentence(s string) string {
	if i := strings.Index(s, ". "); i > 0 {
		return s[:i+1]
	}
	return s
}

var helpRowRE = regexp.MustCompile(`^\s{1,6}(\S+)\s{2,}(\S.*)$`)
var usageBracketRE = regexp.MustCompile("\\[([^\\[\\]]*)\\]")

// optionalPositionalNames extracts the positional names a help text's
// usage block marks optional: the bracketed [name] form (argparse
// [item], cobra add [item]). The usage block is the usage: line plus
// its continuation lines, up to the first blank line; only bracket
// groups there count, so a [word] in prose or a description further
// down cannot flip a positional optional. Inside a group the first
// field is the candidate: a leading dash makes it an optional flag
// group ([-h], [--opt <v>]), not a positional.
func optionalPositionalNames(text string) map[string]bool {
	out := map[string]bool{}
	inUsage := false
	for _, line := range strings.Split(text, "\n") {
		trimmed := strings.TrimSpace(line)
		if strings.HasPrefix(strings.ToLower(trimmed), "usage:") {
			inUsage = true
		} else if inUsage && trimmed == "" {
			inUsage = false
		}
		if !inUsage {
			continue
		}
		for _, m := range usageBracketRE.FindAllStringSubmatch(line, -1) {
			fields := strings.Fields(m[1])
			if len(fields) == 0 || strings.HasPrefix(fields[0], "-") {
				continue
			}
			name := strings.Split(fields[0], "|")[0]
			name = strings.TrimSuffix(strings.Trim(name, "<>[]"), "...")
			if name != "" {
				out[strings.ReplaceAll(name, "-", "_")] = true
			}
		}
	}
	return out
}

// parseFlagSpec parses the flag part of one help or man line:
// "-f, --file <path>", "--output=FILE", "-v". A trailing token that is
// not a flag is the value placeholder.
func parseFlagSpec(spec string) []importFlag {
	spec = strings.TrimSpace(spec)
	// Cut any description the caller left on: two or more spaces after
	// the spec proper is the universal separator.
	if i := strings.Index(spec, "  "); i > 0 {
		spec = spec[:i]
	}
	var f importFlag
	parts := strings.Split(spec, ",")
	for _, part := range parts {
		part = strings.TrimSpace(part)
		fields := strings.Fields(part)
		for j, tok := range fields {
			switch {
			case strings.HasPrefix(tok, "--"):
				name := strings.TrimPrefix(tok, "--")
				// "--flag=value" and "--flag=<value>" name a value;
				// the bracketed "--flag[=<value>]" form names an
				// optional one, which the program accepts only
				// attached — rendering it separated would run a
				// different operation, so the binding is preserved.
				if i := strings.IndexAny(name, "=["); i >= 0 {
					f.Long = name[:i]
					f.Value = strings.Trim(name[i:], "=[]<>")
					f.TakesValue = true
					f.EqualsOnly = name[i] == '['
				} else {
					f.Long = name
				}
			case strings.HasPrefix(tok, "-") && len(tok) == 2:
				f.Short = string(tok[1])
			case j > 0 || (f.Long == "" && f.Short == ""):
				// A bare word after a flag is its value placeholder;
				// before any flag it is not ours.
				if f.Long != "" || f.Short != "" {
					f.Value = tok
					f.TakesValue = true
				}
			}
			if eq := strings.Index(tok, "="); eq >= 0 && strings.HasPrefix(tok, "-") {
				f.TakesValue = true
				if f.Value == "" {
					f.Value = tok[eq+1:]
				}
			}
		}
	}
	if f.Long == "" && f.Short == "" {
		return nil
	}
	return []importFlag{f}
}

func parseHelpText(text string) *importShape {
	shape := &importShape{}
	optional := optionalPositionalNames(text)
	section := ""
	seenGlobal := map[string]bool{}
	var firstUsageLine string
	for _, line := range strings.Split(text, "\n") {
		trimmed := strings.TrimSpace(line)
		lower := strings.ToLower(trimmed)
		switch {
		case strings.HasSuffix(lower, ":") && len(trimmed) < 40:
			switch strings.TrimSuffix(lower, ":") {
			case "commands", "available commands", "subcommands":
				section = "commands"
			case "options", "flags", "optional arguments", "arguments":
				section = "options"
			case "global flags", "global options":
				section = "options"
			case "positional arguments":
				section = "positional"
			default:
				section = ""
			}
			continue
		case strings.HasPrefix(lower, "usage:"):
			section = ""
			if firstUsageLine == "" {
				firstUsageLine = trimmed
			}
			continue
		}
		switch section {
		case "commands":
			m := helpRowRE.FindStringSubmatch(line)
			if m != nil && !strings.HasPrefix(m[1], "-") {
				name := strings.TrimSuffix(m[1], ":")
				if name != "help" && name != "completion" {
					shape.Subs = append(shape.Subs, importSub{Name: name, Desc: strings.TrimSpace(m[2])})
				}
			}
		case "options":
			if strings.HasPrefix(trimmed, "-") {
				spec := trimmed
				desc := ""
				if i := strings.Index(trimmed, "  "); i > 0 {
					spec = trimmed[:i]
					desc = strings.TrimSpace(trimmed[i:])
				}
				for _, f := range parseFlagSpec(spec) {
					f.Desc = desc
					if !seenGlobal[f.flag()] {
						seenGlobal[f.flag()] = true
						shape.Global = append(shape.Global, f)
					}
				}
			}
		case "positional":
			m := helpRowRE.FindStringSubmatch(line)
			if m != nil && !strings.HasPrefix(m[1], "-") {
				name := strings.Trim(m[1], "<>[]")
				shape.Positionals = append(shape.Positionals, importFlag{
					Long:       strings.ReplaceAll(name, "-", "_"),
					Desc:       strings.TrimSpace(m[2]),
					TakesValue: true,
					Optional:   strings.HasPrefix(m[1], "[") || optional[strings.ReplaceAll(name, "-", "_")],
				})
			}
		}
	}
	if shape.Description == "" && firstUsageLine != "" {
		shape.Description = firstUsageLine
	}
	return shape
}
