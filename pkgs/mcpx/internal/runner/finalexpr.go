package runner

import "strings"

// rewriteFinalExpression implements CODE-19 for exec snippets: a
// snippet whose last top-level statement is an expression statement
// has that expression's value as its result, REPL-style, the way
// opencode v2 and Cloudflare's codemode treat a program with no
// return. The rewrite is textual: the final expression is wrapped
// in a call to the generated client's capture global
// (__mcpxFinal(await (<expr>));), which reports the value over the
// record channel as a "final" record, kept apart from stdout and
// from emit() values. The runner applies it only to inline sources
// in the JS harness (never to file scripts, which have the
// default-export entry point, and never when the source declares
// one either -- a program with an entry point already has a result
// rule).
//
// The scanner is deliberately conservative: it tracks comments,
// strings, template literals (with ${} nesting), regex literals
// (by the usual previous-token heuristic) and bracket depth, and
// splits top-level statements at semicolons, at block closes that
// end a block statement, and at newlines where the statement so
// far can plainly end and the next line plainly does not continue
// it. Anything it cannot classify -- an unterminated string or
// regex, a final statement that is a declaration, a block, a
// control-flow head, a label, an import/export -- leaves the
// source untouched: the failure mode of a missed rewrite is
// today's behaviour, never a corrupted program.
func rewriteFinalExpression(src string) (string, bool) {
	spans, hasDefault, ok := scanTopLevel(src)
	if !ok || hasDefault || len(spans) == 0 {
		return src, false
	}
	last := spans[len(spans)-1]
	text := src[last.start:last.end]
	if !isExpressionStatement(text) {
		return src, false
	}
	rest := strings.TrimPrefix(src[last.end:], ";")
	return src[:last.start] + "__mcpxFinal(await (" + text + "));" + rest, true
}

type stmtSpan struct{ start, end int }

var nonExpressionHeads = map[string]bool{
	"import": true, "export": true, "const": true, "let": true,
	"var": true, "function": true, "class": true, "return": true,
	"throw": true, "if": true, "else": true, "for": true,
	"while": true, "do": true, "switch": true, "try": true,
	"catch": true, "finally": true, "break": true, "continue": true,
	"debugger": true,
	// TypeScript declaration heads: exec sources are TypeScript,
	// and a type-level tail is not an expression. A plain
	// identifier with one of these names loses the rewrite --
	// abstention is the safe direction.
	"type": true, "interface": true, "enum": true,
	"namespace": true, "declare": true, "abstract": true,
}

// operandKeywords end an unfinished construct, never a statement:
// a newline after one continues the same statement (new\nC(),
// typeof\nx, const\nanswer = 42), so it is not a boundary.
var operandKeywords = map[string]bool{
	"new": true, "typeof": true, "delete": true, "void": true,
	"await": true, "yield": true, "return": true, "throw": true,
	"case": true, "instanceof": true, "in": true, "of": true,
	"extends": true, "else": true, "do": true,
	"const": true, "let": true, "var": true, "function": true,
	"class": true, "import": true, "export": true,
	// TypeScript assertion operators take a type after them:
	// 1 as\nnumber is one expression, not two statements.
	"as": true, "satisfies": true,
}

// controlHeads open a statement whose condition group is followed
// by a body: a newline between the group and an unbraced body does
// not end the statement.
var controlHeads = map[string]bool{
	"if": true, "for": true, "while": true, "switch": true, "with": true,
}

// blockHeads are the first words of statements a closing brace at
// depth zero can terminate. A depth-zero } after any other
// statement belongs to an expression (an object literal, a
// destructuring), not to a block, so it is not a boundary.
var blockHeads = map[string]bool{
	"if": true, "else": true, "for": true, "while": true, "do": true,
	"try": true, "catch": true, "finally": true, "switch": true,
	"function": true, "class": true,
}

func isExpressionStatement(text string) bool {
	t := strings.TrimSpace(text)
	if t == "" {
		return false
	}
	switch t[0] {
	case '{', '}', ')', ']', ';', '.', ',', '#':
		return false
	}
	word, rest := firstWord(t)
	if word == "" {
		// A literal or an operator-led expression (a string, a
		// number, a unary minus): an expression.
		return true
	}
	if nonExpressionHeads[word] {
		return false
	}
	if word == "async" {
		// Comments may intervene: async /* helper */ function
		// is still a declaration.
		if w2, _ := firstWord(skipTrivia(rest)); w2 == "function" {
			return false
		}
	}
	// A label: word immediately followed by a colon.
	if r := strings.TrimSpace(rest); strings.HasPrefix(r, ":") {
		return false
	}
	return true
}

// firstWord splits a leading identifier from the rest.
func firstWord(s string) (string, string) {
	i := 0
	for i < len(s) && (s[i] == '_' || s[i] == '$' ||
		('a' <= s[i] && s[i] <= 'z') || ('A' <= s[i] && s[i] <= 'Z') ||
		('0' <= s[i] && s[i] <= '9')) {
		i++
	}
	if i == 0 {
		return "", s
	}
	return s[:i], s[i:]
}

// scanTopLevel splits src into top-level statement spans (first
// code token to last code token; surrounding whitespace and
// comments excluded) and reports whether any top-level statement
// is an export default. ok is false on an unterminated construct.
func scanTopLevel(src string) (spans []stmtSpan, hasDefault bool, ok bool) {
	const (
		code = iota
		lineComment
		blockComment
		singleQuote
		doubleQuote
		templateLit
		regexLit
		regexClass
	)
	mode := code
	var braces []byte // '{' plain, '$' for a template ${ frame
	paren, bracket := 0, 0
	templates := 0 // open template literals (we are in code inside ${})
	start := -1    // offset of the current statement's first code token
	lastEnd := -1  // offset just past its last code token
	head := ""     // first word of the current statement
	stmtTokens := 0
	condGroup := false // the statement so far is a control head + its condition group
	lastOperand := false // the last significant token can end an expression
	prevOperand := false

	i := 0
	if strings.HasPrefix(src, "#!") {
		if j := strings.IndexByte(src, '\n'); j >= 0 {
			i = j + 1
		} else {
			return nil, false, true
		}
	}

	depth0 := func() bool {
		return mode == code && paren == 0 && bracket == 0 && len(braces) == 0 && templates == 0
	}
	boundary := func() {
		if start >= 0 && lastEnd > start {
			spans = append(spans, stmtSpan{start, lastEnd})
		}
		start, lastEnd, head = -1, -1, ""
		stmtTokens, condGroup = 0, false
		lastOperand = false
		// A completed statement resets the operand context: a /
		// that opens the next statement is a regex, not division.
		prevOperand = false
	}
	markToken := func(pos, end int) {
		if start < 0 {
			start = pos
		}
		lastEnd = end
		stmtTokens++
		// A token at depth zero after the condition group is the
		// body arriving; the group no longer stands alone.
		if condGroup && stmtTokens > 2 && paren == 0 && len(braces) == 0 && templates == 0 {
			condGroup = false
		}
	}

	for i < len(src) {
		c := src[i]
		switch mode {
		case lineComment:
			if c == '\n' {
				mode = code
			} else {
				i++
				continue
			}
		case blockComment:
			if c == '\n' && start >= 0 && paren == 0 && bracket == 0 && len(braces) == 0 && templates == 0 {
				// A line terminator inside a block comment can
				// separate two statements by ASI, which this
				// scanner does not model: abstain rather than
				// merge them (CODE-19 review, PR #9344).
				return nil, false, false
			}
			if c == '*' && i+1 < len(src) && src[i+1] == '/' {
				mode = code
				i += 2
				continue
			}
			i++
			continue
		case singleQuote, doubleQuote:
			q := byte('\'')
			if mode == doubleQuote {
				q = '"'
			}
			if c == '\\' {
				i += 2
				continue
			}
			if c == q {
				mode = code
				lastEnd = i + 1
				prevOperand = true
				lastOperand = true
				i++
				continue
			}
			if c == '\n' {
				return nil, false, false
			}
			i++
			continue
		case templateLit:
			if c == '\\' {
				i += 2
				continue
			}
			if c == '`' {
				templates--
				mode = code
				lastEnd = i + 1
				prevOperand = true
				lastOperand = true
				i++
				continue
			}
			if c == '$' && i+1 < len(src) && src[i+1] == '{' {
				braces = append(braces, '$')
				templates++
				mode = code
				i += 2
				continue
			}
			i++
			continue
		case regexLit:
			if c == '\\' {
				i += 2
				continue
			}
			if c == '[' {
				mode = regexClass
				i++
				continue
			}
			if c == '/' {
				mode = code
				lastEnd = i + 1
				prevOperand = true
				lastOperand = true
				i++
				continue
			}
			if c == '\n' {
				return nil, false, false
			}
			i++
			continue
		case regexClass:
			if c == '\\' {
				i += 2
				continue
			}
			if c == ']' {
				mode = regexLit
			}
			if c == '\n' {
				return nil, false, false
			}
			i++
			continue
		}

		// code mode
		switch {
		case c == '/' && i+1 < len(src) && src[i+1] == '/':
			mode = lineComment
			i += 2
			continue
		case c == '/' && i+1 < len(src) && src[i+1] == '*':
			mode = blockComment
			i += 2
			continue
		case c == '\'':
			markToken(i, i+1)
			mode = singleQuote
			i++
			continue
		case c == '"':
			markToken(i, i+1)
			mode = doubleQuote
			i++
			continue
		case c == '`':
			markToken(i, i+1)
			templates++
			mode = templateLit
			i++
			continue
		case c == '/' && !prevOperand:
			markToken(i, i+1)
			mode = regexLit
			i++
			continue
		case c == '(':
			markToken(i, i+1)
			lastOperand = false
			if paren == 0 && stmtTokens == 2 && controlHeads[head] {
				condGroup = true
			}
			paren++
			prevOperand = false
			i++
			continue
		case c == ')':
			markToken(i, i+1)
			paren--
			prevOperand = true
			lastOperand = true
			if paren == 0 && condGroup {
				// The condition group of a control head is
				// closed; what follows is a body in statement
				// position, so a / there opens a regex, not a
				// division.
				prevOperand = false
			}
			i++
			continue
		case c == '[':
			markToken(i, i+1)
			lastOperand = false
			bracket++
			prevOperand = false
			i++
			continue
		case c == ']':
			markToken(i, i+1)
			bracket--
			prevOperand = true
			lastOperand = true
			i++
			continue
		case c == '{':
			// Captured before markToken: a { that is the body of
			// a control head is its block, and the body token
			// itself clears condGroup.
			wasCond := condGroup
			markToken(i, i+1)
			lastOperand = false
			if prevOperand || wasCond {
				// After a value or a condition group, { opens a
				// block.
				braces = append(braces, '{')
			} else {
				// In expression position (after =, (, ,, :, an
				// operator, a keyword), { opens an object
				// literal: its close is not a statement boundary.
				braces = append(braces, 'o')
			}
			prevOperand = false
			i++
			continue
		case c == '}':
			markToken(i, i+1)
			lastOperand = true
			if len(braces) > 0 && braces[len(braces)-1] == '$' {
				braces = braces[:len(braces)-1]
				templates--
				mode = templateLit
				i++
				continue
			}
			kind := byte('{')
			if len(braces) > 0 {
				kind = braces[len(braces)-1]
				braces = braces[:len(braces)-1]
			}
			prevOperand = true
			i++
			if kind == '{' && depth0() && blockHeads[head] {
				boundary()
			}
			continue
		case c == ';' && depth0():
			i++
			boundary()
			prevOperand = false
			continue
		case c == '\n' && depth0() && start >= 0:
			// A newline ends the statement only when what precedes
			// can end one and what follows plainly does not continue
			// it (a chained call, a binary operator, an else/catch
			// belonging to the statement above).
			if (canEndAtNewline(src, lastEnd) || lastOperand) && !condGroup && !operandKeywords[lastWordOf(src, lastEnd)] && !continuesOnNextLine(src, i+1) {
				boundary()
			}
			i++
			continue
		case c == ' ' || c == '\t' || c == '\r' || c == '\n':
			i++
			continue
		}

		// An identifier, keyword, number or operator character.
		j := i
		for j < len(src) && (src[j] == '_' || src[j] == '$' ||
			('a' <= src[j] && src[j] <= 'z') || ('A' <= src[j] && src[j] <= 'Z') ||
			('0' <= src[j] && src[j] <= '9')) {
			j++
		}
		if j > i {
			word := src[i:j]
			markToken(i, j)
			if head == "" && start == i {
				head = word
			}
			if depth0() && start == i && word == "export" {
				// Peek the next word: export default ... means the
				// program has an entry point; abstain entirely.
				// Comments may intervene (export /* entry */ default).
				if nextWordAfter(src, j) == "default" {
					hasDefault = true
				}
			}
			if head == "export" && word == "default" {
				// export { answer as default } declares the
				// entry point through a list; same abstention.
				hasDefault = true
			}
			prevOperand = true
			lastOperand = true
			switch word {
			case "return", "typeof", "case", "delete", "void", "in", "of",
				"instanceof", "await", "yield", "new", "else", "do":
				prevOperand = false
			}
			i = j
			continue
		}
		markToken(i, i+1)
		prevOperand = false
		lastOperand = false
		i++
	}
	if mode != code && mode != lineComment {
		return nil, false, false
	}
	boundary()
	return spans, hasDefault, true
}

// lastWordOf returns the identifier the code up to end finishes
// with, or "" when it ends with anything else.
func lastWordOf(src string, end int) string {
	if end > len(src) {
		end = len(src)
	}
	i := end
	for i > 0 && (src[i-1] == '_' || src[i-1] == '$' ||
		('a' <= src[i-1] && src[i-1] <= 'z') || ('A' <= src[i-1] && src[i-1] <= 'Z') ||
		('0' <= src[i-1] && src[i-1] <= '9')) {
		i--
	}
	return src[i:end]
}

// canEndAtNewline reports whether the code up to end can close a
// statement: its last significant character is one an expression
// or declaration can end with.
func canEndAtNewline(src string, end int) bool {
	if end <= 0 || end > len(src) {
		return false
	}
	// A postfix update ends a statement (i++ is complete even
	// though its last character is an operator).
	if end >= 2 && (src[end-2:end] == "++" || src[end-2:end] == "--") {
		return true
	}
	// A TypeScript postfix assertion: obj.answer! is a complete
	// expression even though its last character is an operator.
	if src[end-1] == '!' && end >= 2 {
		c := src[end-2]
		if c == '_' || c == '$' || c == ')' || c == ']' ||
			('a' <= c && c <= 'z') || ('A' <= c && c <= 'Z') || ('0' <= c && c <= '9') {
			return true
		}
	}
	switch src[end-1] {
	case ';', '{', '}', '(', ')', '[', ']':
		return src[end-1] == ')' || src[end-1] == ']' || src[end-1] == '}'
	default:
		c := src[end-1]
		return c == '_' || c == '$' || c == '\'' || c == '"' || c == '`' ||
			('a' <= c && c <= 'z') || ('A' <= c && c <= 'Z') || ('0' <= c && c <= '9')
	}
}

// skipTrivia advances past whitespace (including newlines) and
// comments, returning what remains: the first real token after a
// boundary candidate is what decides continuation, not the next
// physical line -- f() followed by a blank line and (2) is one
// call chain, not two statements.
func skipTrivia(s string) string {
	for {
		s = strings.TrimLeft(s, " \t\r\n")
		if strings.HasPrefix(s, "//") {
			i := strings.IndexByte(s, '\n')
			if i < 0 {
				return ""
			}
			s = s[i+1:]
			continue
		}
		if strings.HasPrefix(s, "/*") {
			i := strings.Index(s, "*/")
			if i < 0 {
				return ""
			}
			s = s[i+2:]
			continue
		}
		return s
	}
}

// nextWordAfter returns the first identifier at or after pos,
// skipping whitespace and comments.
func nextWordAfter(src string, pos int) string {
	w, _ := firstWord(skipTrivia(src[pos:]))
	return w
}

// continuesOnNextLine reports whether the text from pos (just
// after a newline) opens with something that continues the
// previous statement rather than starting a new one.
func continuesOnNextLine(src string, pos int) bool {
	s := skipTrivia(src[pos:])
	if s == "" {
		return false
	}
	// A leading ++ or -- is a prefix update opening a new
	// statement (a restricted production: the newline before it
	// separates), never a binary continuation.
	if strings.HasPrefix(s, "++") || strings.HasPrefix(s, "--") {
		return false
	}
	switch s[0] {
	case '.', '(', '[', '`', '+', '-', '*', '/', '%', '&', '|', '^', '?', ':', ',', '=', '<', '>':
		return true
	}
	if w, _ := firstWord(s); w == "else" || w == "catch" || w == "finally" || w == "while" || w == "instanceof" || w == "in" {
		return true
	}
	return false
}
