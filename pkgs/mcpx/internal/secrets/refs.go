package secrets

import (
	"sort"
)

// References returns the namespaces a script names, sorted and without
// repeats. It finds the spellings that reach a server through the tools
// object, tools.<ns>.<tool> and tools["<ns>"]["<tool>"], and the bare spelling
// <ns>.<tool> for a namespace configured here, which mcpx binds as a global in
// every script it runs.
//
// The scan is static and nothing is evaluated. One lexical pass tokenises
// the source: strings, comments, template text and regex literals are
// skipped (the inside of a template literal's ${...} is read, because a
// call there is still a call), and the declared names are collected from
// that same token stream — declarations, function and arrow parameters,
// destructured bindings and catch bindings — so text inside a comment,
// string, template or regex can never declare a name either. A name the
// script declares itself is not a namespace, so a local called like one
// is not counted. A reference assembled at run time, or a bare use of
// the tools object, is invisible here; the check made when the call
// happens is what catches those.
//
// Known blind spots, kept deliberately: binding is source-global (a name
// bound anywhere counts as declared everywhere, with no block-scoped
// shadowing precision); a "/" directly after a value-closing "}" is read
// as division, so a regex literal starting a statement right after a
// block may be scanned as code; parameters of method shorthand and
// destructuring assignments without a declaration keyword are not
// collected as bindings.
func References(src string, configured func(string) bool) []string {
	toks := lex(src)
	declared := declaredNames(toks)
	found := map[string]bool{}
	for i, t := range toks {
		if t.kind != tIdent {
			continue
		}
		if i > 0 && toks[i-1].kind == tPunct && (toks[i-1].text == "." || toks[i-1].text == "?.") {
			// A member of something else is not a namespace spelling.
			continue
		}
		if t.text == "tools" {
			if i+2 < len(toks) && toks[i+1].text == "." && toks[i+2].kind == tIdent {
				found[toks[i+2].text] = true
			} else if ns, ok := subscriptName(toks, i); ok {
				found[ns] = true
			}
			continue
		}
		if configured(t.text) && !declared[t.text] && i+1 < len(toks) && toks[i+1].text == "." {
			found[t.text] = true
		}
	}
	out := make([]string, 0, len(found))
	for ns := range found {
		out = append(out, ns)
	}
	sort.Strings(out)
	return out
}

// subscriptName reads the bracket spelling at toks[i] ("tools"): the
// namespace is the plain string literal right after "[", and an erased
// TypeScript assertion (as or satisfies) may sit between the
// literal and the closing bracket — tools["fs" as const], tools["fs"!] and
// tools["fs" as Record<string, any>] all reach the same server as
// tools["fs"], so all three count. Anything else (a computed key, an
// expression) is not a literal namespace spelling.
func subscriptName(toks []lexToken, i int) (string, bool) {
	if i+2 >= len(toks) || toks[i+1].text != "[" || toks[i+2].kind != tString || !toks[i+2].plain {
		return "", false
	}
	j := i + 3
	if j < len(toks) && toks[j].text == "!" {
		j++
	} else if j < len(toks) && toks[j].kind == tIdent && (toks[j].text == "as" || toks[j].text == "satisfies") {
		j++
		depth := 0
		closed := false
		for j < len(toks) {
			t := toks[j]
			if t.kind == tPunct {
				if t.text == "]" && depth == 0 {
					closed = true
					break
				}
				switch t.text {
				case "<", "[", "(", "{":
					depth++
				case ">", "]", ")", "}":
					depth--
				case ">>":
					// The lexer emits a nested generic close as
					// one token: it ends two levels, not one
					// (CodeRabbit minor on #1644).
					depth -= 2
				case ">>>":
					depth -= 3
				}
			}
			j++
		}
		if !closed {
			return "", false
		}
	}
	if j < len(toks) && toks[j].text == "]" {
		return toks[i+2].text, true
	}
	return "", false
}

type tokKind int

const (
	tIdent tokKind = iota
	tString
	tNumber
	tTemplate
	tRegex
	tPunct
)

type lexToken struct {
	kind tokKind
	text string
	// plain, for tString, reports the literal had no escape, so its text
	// is exactly the name it spells.
	plain bool
	// nl reports a newline appeared in the whitespace (or a block
	// comment) before this token — the raw material of automatic
	// semicolon insertion.
	nl bool
	// ctrlClose, for a ")" token, reports the parenthesis it closes
	// opened a control condition/head (if/while/for/with/switch), so
	// the ")" does not end a value. It is recorded when the ")" is
	// emitted, from the stack of open-paren contexts — rescanning
	// backward for the matching "(" at every division slash made
	// deeply nested scripts quadratic (Codex, PR #3042).
	ctrlClose bool
}

// exprKeywords are the identifiers after which a value can start, so a
// "/" following one opens a regex literal rather than dividing.
var exprKeywords = map[string]bool{
	"return": true, "typeof": true, "case": true, "in": true, "of": true,
	"instanceof": true, "new": true, "delete": true, "void": true,
	"throw": true, "else": true, "do": true, "await": true, "yield": true,
	"var": true, "let": true, "const": true, "function": true,
	"class": true, "import": true, "extends": true, "default": true,
	// Statement-only keywords: nothing can follow them on their own
	// statement but a label or a terminator, so a "/" after one (the
	// next statement's first token, via ASI) opens a regex, never
	// divides: debugger\n/x*/.test(s), break\n/y/.test(z).
	"debugger": true, "break": true, "continue": true,
}

// controlKeywords are the statement-opening keywords whose following
// parenthesised condition/head does not end a value, so a following "/"
// opens a regex literal rather than dividing: if (true) /regex/.test(s)
var controlKeywords = map[string]bool{
	"if": true, "while": true, "for": true, "with": true, "switch": true,
}

// controlParenOpen reports whether a "(" emitted after toks opens a
// control condition/head: the token before it is a control keyword
// (with the for-await spelling unwrapped), and that keyword is not a
// member name. Evaluated once, when the "(" is emitted.
func controlParenOpen(toks []lexToken) bool {
	if len(toks) == 0 {
		return false
	}
	keyword := len(toks) - 1
	if keyword > 0 && toks[keyword].text == "await" && toks[keyword-1].text == "for" {
		keyword--
	}
	if toks[keyword].kind != tIdent || !controlKeywords[toks[keyword].text] {
		return false
	}
	if keyword > 0 && (toks[keyword-1].text == "." || toks[keyword-1].text == "?.") {
		return false
	}
	return true
}

type lexer struct {
	src       string
	pos       int
	toks      []lexToken
	pendingNL bool
	// parenCtrl is the stack of open-paren contexts: one entry per
	// unmatched "(" token emitted, recording whether that paren
	// opened a control condition/head.
	parenCtrl []bool
}

func lex(src string) []lexToken {
	l := &lexer{src: src}
	l.code(false)
	return l.toks
}

func (l *lexer) emit(kind tokKind, text string, plain bool) {
	tok := lexToken{kind: kind, text: text, plain: plain, nl: l.pendingNL}
	if kind == tPunct && text == "(" {
		l.parenCtrl = append(l.parenCtrl, controlParenOpen(l.toks))
	}
	if kind == tPunct && text == ")" {
		if n := len(l.parenCtrl); n > 0 {
			tok.ctrlClose = l.parenCtrl[n-1]
			l.parenCtrl = l.parenCtrl[:n-1]
		}
	}
	l.toks = append(l.toks, tok)
	l.pendingNL = false
}

// valueEnd reports whether the last emitted token can end a value, which
// is what makes a following "/" a division operator instead of a regex.
func (l *lexer) valueEnd() bool {
	if len(l.toks) == 0 {
		return false
	}
	t := l.toks[len(l.toks)-1]
	switch t.kind {
	case tString, tNumber, tTemplate, tRegex:
		return true
	case tIdent:
		// A member name is a value whatever it spells: o.return ends
		// one even though return alone opens an expression context.
		// The optional-chain spelling counts too: o?.return / 2
		// divides (Codex P2 on #1644).
		if len(l.toks) >= 2 && (l.toks[len(l.toks)-2].text == "." || l.toks[len(l.toks)-2].text == "?.") {
			return true
		}
		if t.text == "const" && len(l.toks) >= 2 && l.toks[len(l.toks)-2].text == "as" {
			// An erased `as const` assertion ends a value even
			// though const alone opens a declaration: in
			// `2 as const / 2` the slash divides.
			return true
		}
		if t.text == "of" {
			// "of" is a keyword only as a for head's separator, where
			// it directly follows the loop target (a name, or a
			// pattern's closing bracket). Anywhere else it is an
			// ordinary name and ends a value: const of = 4; of / 2
			// divides. A value-"of" follows an operator, an opener, a
			// separator or another keyword — never a bare name.
			if len(l.toks) >= 2 {
				prev := l.toks[len(l.toks)-2]
				if (prev.kind == tIdent && !exprKeywords[prev.text]) || prev.text == "]" || prev.text == "}" {
					return false
				}
			}
			return true
		}
		return !exprKeywords[t.text]
	case tPunct:
		if t.text == ")" {
			return !t.ctrlClose
		}
		if t.text == "!" {
			// A postfix non-null assertion (n! / 2) ends a value, so
			// the slash divides; a prefix negation (!/re/.test(s))
			// does not, so the slash opens a regex. Which one this
			// "!" is depends on the token before it.
			return len(l.toks) >= 2 && endsValue(l.toks[len(l.toks)-2])
		}
		return t.text == "]" || t.text == "}" ||
			t.text == "++" || t.text == "--"
	}
	return false
}

// code scans program text. With untilBrace it stops just after the }
// that closes a ${...}; otherwise it scans to the end of the source.
func (l *lexer) code(untilBrace bool) {
	depth := 0
	for l.pos < len(l.src) {
		c := l.src[l.pos]
		switch {
		case c == ' ' || c == '\t' || c == '\n' || c == '\r':
			if c == '\n' {
				l.pendingNL = true
			}
			l.pos++
		case c == '/' && l.pos+1 < len(l.src) && l.src[l.pos+1] == '/':
			l.pos = l.lineEnd(l.pos)
		case c == '/' && l.pos+1 < len(l.src) && l.src[l.pos+1] == '*':
			end := l.blockEnd(l.pos)
			for _, ch := range l.src[l.pos:end] {
				if ch == '\n' {
					l.pendingNL = true
				}
			}
			l.pos = end
		case c == '/':
			if l.valueEnd() {
				l.emit(tPunct, "/", false)
				l.pos++
			} else {
				l.regex()
			}
		case c == '\'' || c == '"':
			l.quoted()
		case c == '`':
			l.template()
		case c == '{':
			depth++
			l.emit(tPunct, "{", false)
			l.pos++
		case c == '}':
			l.emit(tPunct, "}", false)
			l.pos++
			if untilBrace && depth == 0 {
				return
			}
			depth--
		case c >= '0' && c <= '9':
			j := l.pos
			hasDot := false
			for j < len(l.src) {
				if isIdentByte(l.src[j]) {
					j++
				} else if l.src[j] == '.' && !hasDot {
					hasDot = true
					j++
				} else {
					break
				}
			}
			l.emit(tNumber, l.src[l.pos:j], false)
			l.pos = j
		case isIdentStart(c):
			j := l.pos
			for j < len(l.src) && isIdentByte(l.src[j]) {
				j++
			}
			l.emit(tIdent, l.src[l.pos:j], false)
			l.pos = j
		default:
			l.punct()
		}
	}
}

// regex consumes a regex literal opening at l.pos, honouring escapes and
// character classes, plus any flags after the closing slash.
func (l *lexer) regex() {
	j := l.pos + 1
	inClass := false
	for j < len(l.src) {
		switch l.src[j] {
		case '\\':
			j += 2
			continue
		case '[':
			inClass = true
		case ']':
			inClass = false
		case '/':
			if !inClass {
				j++
				for j < len(l.src) && isIdentByte(l.src[j]) {
					j++
				}
				l.emit(tRegex, l.src[l.pos:j], false)
				l.pos = j
				return
			}
		case '\n':
			l.emit(tRegex, l.src[l.pos:j], false)
			l.pos = j
			return
		}
		j++
	}
	l.emit(tRegex, l.src[l.pos:], false)
	l.pos = len(l.src)
}

// quoted consumes a '...' or "..." literal, emitting its raw inner text.
func (l *lexer) quoted() {
	q := l.src[l.pos]
	plain := true
	for j := l.pos + 1; j < len(l.src); j++ {
		switch l.src[j] {
		case '\\':
			plain = false
			j++
		case q:
			l.emit(tString, l.src[l.pos+1:j], plain)
			l.pos = j + 1
			return
		case '\n':
			// An unterminated quote ends at its line, as the language does.
			l.emit(tString, l.src[l.pos+1:j], plain)
			l.pos = j
			return
		}
	}
	l.emit(tString, l.src[l.pos+1:], plain)
	l.pos = len(l.src)
}

// template consumes a template literal, lexing the program inside each
// ${...} as code between synthetic { } tokens.
func (l *lexer) template() {
	l.pos++
	for l.pos < len(l.src) {
		switch l.src[l.pos] {
		case '\\':
			l.pos += 2
		case '`':
			l.pos++
			l.emit(tTemplate, "", false)
			return
		case '$':
			if l.pos+1 < len(l.src) && l.src[l.pos+1] == '{' {
				l.emit(tPunct, "{", false)
				l.pos += 2
				l.code(true)
				continue
			}
			l.pos++
		default:
			l.pos++
		}
	}
	l.emit(tTemplate, "", false)
}

var puncts = []string{
	"...", "===", "!==", "**=", "<<=", ">>=", "&&=", "||=", "??=", ">>>",
	"=>", "==", "!=", "<=", ">=", "&&", "||", "??", "?.", "++", "--",
	"+=", "-=", "*=", "/=", "%=", "**", "<<", ">>", "&=", "|=", "^=",
}

func (l *lexer) punct() {
	rest := l.src[l.pos:]
	for _, p := range puncts {
		if len(rest) >= len(p) && rest[:len(p)] == p {
			l.emit(tPunct, p, false)
			l.pos += len(p)
			return
		}
	}
	l.emit(tPunct, rest[:1], false)
	l.pos++
}

func (l *lexer) lineEnd(i int) int {
	for j := i; j < len(l.src); j++ {
		if l.src[j] == '\n' {
			return j
		}
	}
	return len(l.src)
}

func (l *lexer) blockEnd(i int) int {
	for j := i + 2; j+1 < len(l.src); j++ {
		if l.src[j] == '*' && l.src[j+1] == '/' {
			return j + 2
		}
	}
	return len(l.src)
}

// declaredNames collects the names the source binds for itself from the
// token stream: declaration keywords, function and arrow parameters,
// destructured bindings (plain and renamed) and catch bindings. A bare
// use of one of them is a local, whatever namespace it shares a name with.
func declaredNames(toks []lexToken) map[string]bool {
	out := map[string]bool{}
	angles := angleEnds(toks)
	parens := parenMatches(toks)
	typeOnly := typeAliasMask(toks)
	forHead := forHeadDecls(toks)
	for i, t := range toks {
		if typeOnly[i] {
			// A type alias is erased at runtime: nothing inside it
			// binds a name, including a function type's parameters.
			continue
		}
		if t.kind == tPunct && t.text == "(" {
			// An arrow function's parameter list: (...) => ..., or
			// the typed spelling (...): Type => ...
			if close := parens[i]; close >= 0 && arrowAfterParams(toks, close) {
				if !isAnnotationTypeParen(toks, i) {
					parseParams(toks, i, out, angles)
				}
			}
			continue
		}
		if t.kind != tIdent {
			continue
		}
		if i > 0 && toks[i-1].kind == tPunct && (toks[i-1].text == "." || toks[i-1].text == "?.") {
			continue
		}
		// A lone identifier before => is an arrow parameter —
		// unless it is the return-type annotation of a
		// parenthesised parameter list (`(): demo => ...`): the
		// annotation vanishes at runtime and binds nothing, so
		// recording it would shadow the real namespace use.
		if i+1 < len(toks) && toks[i+1].kind == tPunct && toks[i+1].text == "=>" {
			isReturnType := i >= 2 && toks[i-1].kind == tPunct && toks[i-1].text == ":" &&
				toks[i-2].kind == tPunct && toks[i-2].text == ")"
			if !isReturnType {
				out[t.text] = true
			}
		}
		switch t.text {
		case "const", "let", "var":
			// A property key is not a declaration: { let: 1, demo: 2 }
			// binds nothing, and entering parseDeclarators here would
			// record the sibling keys as bindings.
			if i+1 < len(toks) && toks[i+1].kind == tPunct && toks[i+1].text == ":" {
				continue
			}
			parseDeclarators(toks, i+1, out, forHead[i], angles)
		case "function":
			j := i + 1
			if j < len(toks) && toks[j].text == "*" {
				j++
			}
			if j < len(toks) && toks[j].kind == tIdent {
				out[toks[j].text] = true
				j++
			}
			if j < len(toks) && toks[j].text == "(" {
				parseParams(toks, j, out, angles)
			}
		case "class":
			if i+1 < len(toks) && toks[i+1].kind == tIdent {
				out[toks[i+1].text] = true
			}
		case "import":
			parseImport(toks, i+1, out)
		case "catch":
			if i+2 < len(toks) && toks[i+1].text == "(" {
				parseTarget(toks, i+2, out, angles)
			}
		}
	}
	return out
}

// arrowAfterParams reports whether the token just past a parameter
// list's closing paren (index close) makes the list an arrow
// function's: either a direct "=>", or a return-type annotation in
// between — (demo): string => ... binds demo exactly like the
// untyped spelling (CodeRabbit minor on #1644). The annotation walk
// mirrors skipAnnotation's nesting, except that a "=>" at the
// annotation's own level ends it: a function-type annotation's own
// arrow is always parenthesised or angled, so it never sits at
// that level.
func arrowAfterParams(toks []lexToken, close int) bool {
	i := close + 1
	if i >= len(toks) {
		return false
	}
	if toks[i].kind == tPunct && toks[i].text == "=>" {
		return true
	}
	if toks[i].kind != tPunct || toks[i].text != ":" {
		return false
	}
	depth, angle := 0, 0
	for i++; i < len(toks); i++ {
		t := toks[i]
		if t.kind != tPunct {
			continue
		}
		switch t.text {
		case "(", "[", "{":
			depth++
		case ")", "]", "}":
			if depth == 0 {
				return false
			}
			depth--
		case "<":
			angle++
		case ">":
			if angle > 0 {
				angle--
			}
		case ">>":
			angle -= 2
			if angle < 0 {
				angle = 0
			}
		case ">>>":
			angle -= 3
			if angle < 0 {
				angle = 0
			}
		case "=>":
			if depth == 0 && angle == 0 {
				return true
			}
		case ",", "=", ";":
			if depth == 0 && angle == 0 {
				return false
			}
		}
	}
	return false
}

// parenMatches computes, for every "(" token, the index of the token
// that returns its bracket depth to zero, in one stack pass over the
// stream. A bracket of any kind opens a level and any closer ends the
// most recent level — the same pairing the former per-paren depth scan
// produced — but precomputing keeps declaredNames linear: rescanning
// the suffix at every "(" made deeply nested scripts quadratic.
// matches[i] is -1 unless token i is "(" and its level is closed by ")".
func parenMatches(toks []lexToken) []int {
	matches := make([]int, len(toks))
	for i := range matches {
		matches[i] = -1
	}
	var stack []int
	for i, t := range toks {
		if t.kind != tPunct {
			continue
		}
		switch t.text {
		case "(", "[", "{":
			stack = append(stack, i)
		case ")", "]", "}":
			if len(stack) == 0 {
				continue
			}
			open := stack[len(stack)-1]
			stack = stack[:len(stack)-1]
			if toks[open].text == "(" && t.text == ")" {
				matches[open] = i
			}
		}
	}
	return matches
}

// skipExpr advances past an expression, stopping before a token in stops
// (or a closing bracket) at the expression's own nesting depth.
func skipExpr(toks []lexToken, i int, stops map[string]bool, angles []int) int {
	depth := 0
	for i < len(toks) {
		// Only punctuation delimits: a literal whose text happens to
		// spell a delimiter (the string ",", a template, a regex) is
		// expression content, never structure.
		if toks[i].kind != tPunct {
			i++
			continue
		}
		t := toks[i].text
		if depth == 0 && t == "<" && i > 0 && toks[i-1].kind == tIdent && angles[i] >= 0 && angles[i] < len(toks) && toks[angles[i]].text == "(" {
			i = angles[i]
			continue
		}
		// An uncalled instantiation expression in the expression
		// (a parameter default above all: x = identity<string,
		// demo>) is the same type list with no call parenthesis:
		// jump it under the same guard skipDeclarator uses, so its
		// commas are not expression separators (Codex P2 on #1644).
		if depth == 0 && t == "<" && i > 0 && toks[i-1].kind == tIdent && jumpableAngleList(toks, i, angles) {
			i = angles[i]
			continue
		}
		if depth == 0 && stops[t] {
			return i
		}
		switch t {
		case "(", "[", "{":
			depth++
		case ")", "]", "}":
			if depth == 0 {
				return i
			}
			depth--
		}
		i++
	}
	return i
}

// parseTarget reads one binding target at i — an identifier or a
// destructuring pattern — recording every name it binds, and returns the
// index just past it.
func parseTarget(toks []lexToken, i int, out map[string]bool, angles []int) int {
	if i >= len(toks) {
		return i
	}
	switch {
	case toks[i].kind == tIdent:
		out[toks[i].text] = true
		return i + 1
	case toks[i].text == "{":
		i++
		// Only punctuation delimits the pattern: a literal whose
		// text spells a delimiter (the string "}", a template ",")
		// is a property key, never structure — the same rule
		// skipExpr follows.
		for i < len(toks) && !(toks[i].kind == tPunct && toks[i].text == "}") {
			switch {
			case toks[i].kind == tPunct && toks[i].text == ",":
				i++
			case toks[i].kind == tPunct && toks[i].text == "...":
				i++
				if i < len(toks) && toks[i].kind == tIdent {
					out[toks[i].text] = true
					i++
				}
			case toks[i].kind == tPunct && toks[i].text == "[":
				// A computed key: skip it, then the target after ':'.
				i = skipExpr(toks, i, map[string]bool{":": true}, angles)
				if i < len(toks) && toks[i].text == ":" {
					i = parseTarget(toks, i+1, out, angles)
				}
			case toks[i].kind == tIdent || toks[i].kind == tString || toks[i].kind == tNumber:
				key := toks[i]
				i++
				if i < len(toks) && toks[i].text == ":" {
					i = parseTarget(toks, i+1, out, angles)
				} else if key.kind == tIdent {
					// Shorthand: the key is the binding.
					out[key.text] = true
				}
			default:
				i++
			}
			if i < len(toks) && toks[i].text == "=" {
				i = skipExpr(toks, i+1, map[string]bool{",": true, "}": true}, angles)
			}
		}
		if i < len(toks) {
			i++
		}
		return i
	case toks[i].text == "[":
		i++
		for i < len(toks) && !(toks[i].kind == tPunct && toks[i].text == "]") {
			if toks[i].kind == tPunct && toks[i].text == "," {
				i++
				continue
			}
			if toks[i].kind == tPunct && toks[i].text == "..." {
				i++
			}
			i = parseTarget(toks, i, out, angles)
			if i < len(toks) && toks[i].text == "=" {
				i = skipExpr(toks, i+1, map[string]bool{",": true, "]": true}, angles)
			}
		}
		if i < len(toks) {
			i++
		}
		return i
	}
	return i + 1
}

// parseParams records the bindings of the parameter list opening at i.
func parseParams(toks []lexToken, i int, out map[string]bool, angles []int) {
	i++
	for i < len(toks) && toks[i].text != ")" {
		if toks[i].text == "," {
			i++
			continue
		}
		if toks[i].text == "..." {
			i++
		}
		i = parseTarget(toks, i, out, angles)
		if i < len(toks) && toks[i].kind == tPunct && toks[i].text == ":" {
			// A TypeScript annotation is not more bindings:
			// f(x: typeof demo) binds x alone.
			i = skipAnnotation(toks, i+1)
		}
		if i < len(toks) && toks[i].text == "=" {
			i = skipExpr(toks, i+1, map[string]bool{",": true, ")": true}, angles)
		}
	}
}

// skipAnnotation advances past a TypeScript type annotation, stopping
// before the token that ends it at its own level: a comma, the closing
// paren, or a default's "=". Brackets and generic angle brackets nest.
func skipAnnotation(toks []lexToken, i int) int {
	depth, angle := 0, 0
	for i < len(toks) {
		t := toks[i]
		if t.kind == tPunct {
			switch t.text {
			case "(", "[", "{":
				depth++
			case ")", "]", "}":
				if depth == 0 {
					return i
				}
				depth--
			case "<":
				angle++
			case ">":
				if angle > 0 {
					angle--
				}
			case ">>":
				angle -= 2
				if angle < 0 {
					angle = 0
				}
			case ">>>":
				angle -= 3
				if angle < 0 {
					angle = 0
				}
			case ",", "=", ";":
				if depth == 0 && angle == 0 {
					return i
				}
			}
		}
		i++
	}
	return i
}

// forHeadDecls marks every const/let/var keyword that opens a for
// statement's head, in ONE forward pass over the token stream: a
// declaration qualifies when the innermost unmatched "(" around it
// belongs to a "for" and no ";" has closed the head's first part
// yet. In a for head, "in"/"of" at the head's own level separates
// the bindings from the iterated expression — it never continues an
// initialiser — so declarator collection must stop there:
// for (const x in {}, demo.echo()) binds x alone, and the comma in
// the right-hand expression is not a declaration separator.
// (A per-declaration backward rescan here was quadratic on
// declaration-heavy scripts — Codex round 5 on PR #3042.)
func forHeadDecls(toks []lexToken) []bool {
	out := make([]bool, len(toks))
	type paren struct {
		isFor bool
		semi  bool
	}
	var stack []paren
	for i, t := range toks {
		if t.kind == tPunct {
			switch t.text {
			case "(":
				keyword := i - 1
				if keyword > 0 && toks[keyword].text == "await" && toks[keyword-1].text == "for" {
					keyword--
				}
				isFor := keyword >= 0 && toks[keyword].kind == tIdent && toks[keyword].text == "for" &&
					(keyword == 0 || (toks[keyword-1].text != "." && toks[keyword-1].text != "?."))
				stack = append(stack, paren{isFor: isFor})
			case ")":
				if len(stack) > 0 {
					stack = stack[:len(stack)-1]
				}
			case ";":
				if len(stack) > 0 {
					stack[len(stack)-1].semi = true
				}
			}
			continue
		}
		if t.kind == tIdent && (t.text == "const" || t.text == "let" || t.text == "var") &&
			len(stack) > 0 && stack[len(stack)-1].isFor && !stack[len(stack)-1].semi {
			out[i] = true
		}
	}
	return out
}

// isAnnotationTypeParen reports whether the parenthesised list at i,
// followed by "=>", is a function TYPE in an annotation rather than a
// runtime arrow: a variable annotation (let g: (demo: string) =>
// string) or a return annotation (function f(): (demo) => string).
// An object-literal value ({ f: (demo) => ... }) is a runtime arrow:
// its colon is preceded by "{" or ",", never by a declaration keyword
// or a parameter list's ")".
func isAnnotationTypeParen(toks []lexToken, i int) bool {
	if i < 2 || toks[i-1].text != ":" {
		return false
	}
	prev := toks[i-2]
	if prev.kind == tPunct && prev.text == ")" {
		return true
	}
	if prev.kind == tIdent && i >= 3 && toks[i-3].kind == tIdent {
		switch toks[i-3].text {
		case "const", "let", "var":
			return true
		}
	}
	return false
}

// skipAngles returns the index just past a generic angle-bracket list
// opening at i, or -1 when the brackets do not balance.
func skipAngles(toks []lexToken, i int) int {
	angle := 0
	for j := i; j < len(toks); j++ {
		t := toks[j]
		if t.kind != tPunct {
			continue
		}
		switch t.text {
		case "<":
			angle++
		case ">":
			angle--
		case ">>":
			angle -= 2
		case ">>>":
			angle -= 3
		}
		if angle < 0 {
			return -1
		}
		if angle == 0 {
			return j + 1
		}
	}
	return -1
}

// typeAliasMask marks the tokens of every type alias declaration
// (type Name<...> = ... ;) — a region that is erased at runtime and
// therefore binds nothing, function-type parameters included. The
// region ends at its ";" at the alias's own level, at an enclosing
// block's "}", or at an ASI boundary, so a following statement is
// never swallowed.
func typeAliasMask(toks []lexToken) []bool {
	mask := make([]bool, len(toks))
	for i := 0; i+2 < len(toks); i++ {
		if mask[i] || toks[i].kind != tIdent || toks[i].text != "type" {
			continue
		}
		if i > 0 {
			p := toks[i-1]
			if p.kind == tIdent && p.text == "export" {
				// An exported alias (export type F = ...) is the
				// same erased region; the modifier before it must
				// itself sit at a statement boundary.
				if i > 1 {
					q := toks[i-2]
					if !(q.kind == tPunct && (q.text == ";" || q.text == "{" || q.text == "}")) {
						continue
					}
				}
			} else if !(p.kind == tPunct && (p.text == ";" || p.text == "{" || p.text == "}")) {
				continue
			}
		}
		if toks[i+1].kind != tIdent {
			continue
		}
		j := i + 2
		if j < len(toks) && toks[j].text == "<" {
			if e := skipAngles(toks, j); e >= 0 {
				j = e
			}
		}
		if j >= len(toks) || toks[j].text != "=" {
			continue
		}
		depth, angle := 0, 0
		end := len(toks)
		for k := i; k < len(toks); k++ {
			t := toks[k]
			if k > i && depth == 0 && t.kind == tPunct && t.text == "}" {
				end = k
				break
			}
			// ASI boundary: a newline after a token that can end the
			// alias's type — a value token, or the ">" closing a
			// generic (type F = Array<string>\nconst demo = ...) —
			// before a token that can only start a new statement.
			// Inside "<...>" a newline is just type layout, never a
			// boundary, so the check requires angle == 0.
			if k > i && depth == 0 && angle == 0 && t.nl && startsStatement(t) &&
				(endsValue(toks[k-1]) || toks[k-1].text == ">" || toks[k-1].text == ">>" || toks[k-1].text == ">>>") {
				end = k
				break
			}
			if t.kind != tPunct {
				continue
			}
			switch t.text {
			case "(", "[", "{":
				depth++
			case ")", "]", "}":
				depth--
			case "<":
				angle++
			case ">":
				angle--
			case ">>":
				angle -= 2
			case ">>>":
				angle -= 3
			case ";":
				if depth == 0 {
					end = k + 1
					k = len(toks)
				}
			}
		}
		for k := i; k < end && k < len(toks); k++ {
			mask[k] = true
		}
		if end > i {
			i = end - 1
		}
	}
	return mask
}

// parseDeclarators records the bindings of a const/let/var statement
// whose first declarator starts at i.
func parseDeclarators(toks []lexToken, i int, out map[string]bool, forHead bool, angles []int) {
	for i < len(toks) {
		i = parseTarget(toks, i, out, angles)
		if i < len(toks) && toks[i].kind == tPunct && toks[i].text == ":" {
			i = skipAnnotation(toks, i+1)
		}
		i = skipDeclarator(toks, i, forHead, angles)
		if i < len(toks) && toks[i].text == "," {
			i++
			continue
		}
		return
	}
}

// skipDeclarator advances past one declarator's initialiser, stopping
// before a depth-0 comma or semicolon — or at an automatic-semicolon
// boundary: a newline after a value-ending token, before a token that
// can only start a new statement. Without that stop, semicolon-free
// code (const x = 1\na(), demo.echo()) lets the next statement's
// comma pose as a declaration separator and its first name as a
// binding.
// angleEnds computes balanced angle-list ends in linear time. A multi-token
// close balances only the outermost opening it consumes.
func angleEnds(toks []lexToken) []int {
	ends := make([]int, len(toks))
	for i := range ends {
		ends[i] = -1
	}
	var stack []int
	for i, t := range toks {
		if t.kind != tPunct {
			continue
		}
		if t.text == "<" {
			stack = append(stack, i)
			continue
		}
		n := 0
		switch t.text {
		case ">":
			n = 1
		case ">>":
			n = 2
		case ">>>":
			n = 3
		}
		if n == 0 {
			continue
		}
		if len(stack) < n {
			stack = stack[:0]
			continue
		}
		open := stack[len(stack)-n]
		stack = stack[:len(stack)-n]
		ends[open] = i + 1
	}
	return ends
}

// jumpableAngleList reports whether the angle list opening at i is
// balanced and its interior can only be type arguments, never a
// comparison chain spanning a real separator: no "=" (a declarator's
// or assignment's equals inside the span would prove the "<" was a
// comparison) and no ";" appears between the brackets.
func jumpableAngleList(toks []lexToken, i int, angles []int) bool {
	end := angles[i]
	if end < 0 || end > len(toks) {
		return false
	}
	for k := i + 1; k < end-1; k++ {
		if toks[k].kind == tPunct && (toks[k].text == "=" || toks[k].text == ";") {
			return false
		}
	}
	return true
}

func skipDeclarator(toks []lexToken, i int, forHead bool, angles []int) int {
	depth := 0
	for i < len(toks) {
		t := toks[i]
		// A closing ">" of a type-argument list also ends the
		// declarator's value for ASI purposes — endsValue does
		// not claim it (a ">" can be a comparison), but at a
		// newline after an angle list the same ruling the type
		// alias mask uses applies (CodeRabbit minor on #1644).
		if depth == 0 && t.nl && i > 0 && (endsValue(toks[i-1]) || toks[i-1].text == ">" || toks[i-1].text == ">>" || toks[i-1].text == ">>>") && startsStatement(t) {
			return i
		}
		if forHead && depth == 0 && t.kind == tIdent && (t.text == "in" || t.text == "of") &&
			!(i > 0 && toks[i-1].text == ".") {
			return i
		}
		if t.kind != tPunct {
			i++
			continue
		}
		// A generic call's type arguments sit at this declarator's
		// own depth, and their commas are not declaration
		// boundaries: identity<string, demo>("") binds nothing.
		// Jump the whole list when it has the call shape — and the
		// instantiation-expression shape too: identity<string, demo>
		// stored, not called, is the same type list with no call
		// parenthesis after it. The no-call jump is deliberately
		// narrower (jumpableAngleList): a "<" that is really a
		// comparison can only span a real declarator separator when
		// the tokens between hold that declarator's "=" — a type
		// list never does.
		if depth == 0 && t.text == "<" && i > 0 && toks[i-1].kind == tIdent {
			if end := angles[i]; end >= 0 && end < len(toks) && toks[end].kind == tPunct && toks[end].text == "(" {
				i = end
				continue
			}
			if jumpableAngleList(toks, i, angles) {
				i = angles[i]
				continue
			}
		}
		// A type assertion's arguments have no call parenthesis —
		// {} as Record<string, demo>, {} satisfies Map<string,
		// demo> — but the type is still not bindings: jump any
		// "<...>" whose type name follows as/satisfies.
		if depth == 0 && t.text == "<" && i > 1 && toks[i-1].kind == tIdent &&
			toks[i-2].kind == tIdent && (toks[i-2].text == "as" || toks[i-2].text == "satisfies") {
			if end := angles[i]; end >= 0 {
				i = end
				continue
			}
		}
		if depth == 0 && (t.text == "," || t.text == ";") {
			return i
		}
		switch t.text {
		case "(", "[", "{":
			depth++
		case ")", "]", "}":
			if depth == 0 {
				return i
			}
			depth--
		}
		i++
	}
	return i
}

// endsValue reports whether a token can end a statement's value, for
// the ASI boundary test in skipDeclarator.
func endsValue(t lexToken) bool {
	switch t.kind {
	case tString, tNumber, tTemplate, tRegex:
		return true
	case tIdent:
		return !exprKeywords[t.text]
	case tPunct:
		return t.text == ")" || t.text == "]" || t.text == "}" ||
			t.text == "++" || t.text == "--"
	}
	return false
}

// startsStatement reports whether a token can only begin a new
// statement (never continue the previous expression), for the ASI
// boundary test in skipDeclarator.
func startsStatement(t lexToken) bool {
	switch t.kind {
	case tIdent, tString, tNumber, tTemplate, tRegex:
		return true
	}
	return false
}

// parseImport records the local names an import statement binds.
func parseImport(toks []lexToken, i int, out map[string]bool) {
	if i >= len(toks) || toks[i].text == "(" || toks[i].kind == tString {
		return
	}
	if toks[i].kind == tIdent {
		// A whole-statement type import (import type ...) binds no runtime values.
		if toks[i].text == "type" && i+1 < len(toks) {
			next := toks[i+1]
			if next.text == "{" || next.text == "*" || (next.kind == tIdent && next.text != "from") {
				return
			}
		}
		out[toks[i].text] = true
		i++
		if i < len(toks) && toks[i].text == "," {
			i++
		}
	}
	if i < len(toks) && toks[i].text == "*" {
		i++
		if i < len(toks) && toks[i].text == "as" {
			i++
		}
		if i < len(toks) && toks[i].kind == tIdent {
			out[toks[i].text] = true
		}
		return
	}
	if i < len(toks) && toks[i].text == "{" {
		i++
		for i < len(toks) && toks[i].text != "}" {
			if toks[i].text == "," {
				i++
				continue
			}
			// An inline type-only specifier: "type <ident>" or "type <string>"
			if toks[i].kind == tIdent && toks[i].text == "type" && i+1 < len(toks) &&
				(toks[i+1].kind == tIdent || toks[i+1].kind == tString) &&
				toks[i+1].text != "as" && toks[i+1].text != "," && toks[i+1].text != "}" {
				i += 2
				if i < len(toks) && toks[i].text == "as" {
					i++
					if i < len(toks) && (toks[i].kind == tIdent || toks[i].kind == tString) {
						i++
					}
				}
				continue
			}
			if toks[i].kind == tIdent || toks[i].kind == tString {
				key := toks[i]
				i++
				if i < len(toks) && toks[i].text == "as" {
					i++
					if i < len(toks) && toks[i].kind == tIdent {
						out[toks[i].text] = true
						i++
					}
				} else if key.kind == tIdent {
					out[key.text] = true
				}
				continue
			}
			i++
		}
	}
}

func isIdentStart(b byte) bool {
	return b == '_' || b == '$' || (b >= 'a' && b <= 'z') || (b >= 'A' && b <= 'Z') || b >= 0x80
}

func isIdentByte(b byte) bool {
	return isIdentStart(b) || (b >= '0' && b <= '9')
}
