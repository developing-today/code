package scripting

import (
	"fmt"
	"strings"
	"unicode"

	"github.com/dezren39/mcpx/internal/diagnose"
)

// ClarificationQuestion represents a prompt to ask a user or agent when a script
// has ambiguous or missing parameters.
type ClarificationQuestion struct {
	Tool     string   `json:"tool,omitempty"`
	Field    string   `json:"field,omitempty"`
	Question string   `json:"question"`
	Options  []string `json:"options,omitempty"`
}

// ValidationResult summarizes the static and schema checks on a script.
type ValidationResult struct {
	Valid          bool                    `json:"valid"`
	Diagnostics    []diagnose.Diagnostic   `json:"diagnostics,omitempty"`
	Questions      []ClarificationQuestion `json:"questions,omitempty"`
	SuggestedFixes []string                `json:"suggestedFixes,omitempty"`
}

// HasFatal reports whether any diagnostic is marked fatal.
func (v ValidationResult) HasFatal() bool {
	for _, d := range v.Diagnostics {
		if d.Fatal {
			return true
		}
	}
	return false
}

// Summary returns a human-readable synopsis of the validation result.
func (v ValidationResult) Summary() string {
	if v.Valid && len(v.Diagnostics) == 0 {
		return "script is valid with no diagnostics"
	}
	var sb strings.Builder
	if v.Valid {
		sb.WriteString("script is valid with non-fatal warnings:\n")
	} else {
		sb.WriteString("script validation failed:\n")
	}
	for _, d := range v.Diagnostics {
		sb.WriteString("  - ")
		sb.WriteString(d.String())
		sb.WriteString("\n")
	}
	if len(v.Questions) > 0 {
		sb.WriteString("\nClarifying questions:\n")
		for _, q := range v.Questions {
			sb.WriteString(fmt.Sprintf("  ? [%s] %s\n", q.Field, q.Question))
		}
	}
	return strings.TrimSpace(sb.String())
}

// Validate checks a script for obvious syntax/delimiter errors and compares
// all tool calls against the available catalog schemas.
// Crucial guarantee: It never marks a valid script as invalid.
// The delimiter scanner tells regex literals from division with a
// previous-significant-token heuristic (keywords, control headers, block
// closures and TypeScript postfix assertions included); see
// checkSyntaxAndDelimiters for the pinned blind spots that remain.
func Validate(src string, cat *diagnose.Catalog) ValidationResult {
	res := ValidationResult{Valid: true}

	// 1. Delimiter & Syntax pre-flight check
	synDiags := checkSyntaxAndDelimiters(src)
	res.Diagnostics = append(res.Diagnostics, synDiags...)

	// 2. Catalog and tool schema checks
	if cat != nil {
		toolDiags := diagnose.Script(src, *cat)
		res.Diagnostics = append(res.Diagnostics, toolDiags...)
	}

	// 3. Process diagnostics into questions & suggested fixes
	for _, d := range res.Diagnostics {
		if d.Fatal {
			res.Valid = false
		}
		if d.Kind == diagnose.KindMissingArgument {
			res.Questions = append(res.Questions, ClarificationQuestion{
				Tool:     d.Tool,
				Field:    d.Field,
				Question: fmt.Sprintf("Missing required parameter %q for tool %s: what value should be used?", d.Field, d.Tool),
			})
		}
		if d.Fix != "" {
			res.SuggestedFixes = append(res.SuggestedFixes, d.Fix)
		}
	}

	return res
}

type delimToken struct {
	char      rune
	line      int
	exprAfter bool
}

func checkSyntaxAndDelimiters(src string) []diagnose.Diagnostic {
	var diags []diagnose.Diagnostic
	var stack []delimToken

	line := 1
	closedExprAfter := false
	inSingleLineComment := false
	inMultiLineComment := false
	inString := rune(0)
	isEscaped := false
	inRegex := false
	inCharClass := false
	regexEscaped := false
	// prevSig classifies the previous significant (non-whitespace,
	// non-comment) token for the regex-vs-division ruling in the '/'
	// handling below: 0 = none yet, 'o' = operator or opening delimiter
	// (expression position: a '/' opens a regex), 'x' = operand —
	// identifier, number, string, regex, or closing delimiter (a '/'
	// there is division).
	prevSig := rune(0)
	// signAfterOperand records that the previous significant
	// character was a '+' or '-' seen while prevSig was 'x', so an
	// immediately adjacent identical sign completes a postfix
	// ++/-- — an operand — rather than two operators (Codex P2 on
	// #1659: n++ / 2 divides, but the second '+' flipped prevSig
	// to expression position and the slash opened a phantom
	// regex).
	signAfterOperand := false
	var curIdent []rune
	lastIdent := ""
	prevChar := rune(0)
	// identPrev is the last significant character before the
	// identifier currently being accumulated started, and
	// lastIdentPrev the same for lastIdent. Keyword classification
	// must consult the character BEFORE the identifier: by the
	// time an identifier completes, prevChar holds its own final
	// character, so a member property named like a keyword
	// (obj.return, obj.catch()) was misread as the keyword itself
	// (Codex P2 on #1659).
	identPrev := rune(0)
	lastIdentPrev := rune(0)
	// prevIdent is the completed identifier before the one now
	// being accumulated, captured when an identifier starts; it
	// spots expression keywords in front of function/class
	// (return function() {} / 2 divides after the body closure).
	prevIdent := ""
	// expectFuncClassBody is set while scanning a function or
	// class EXPRESSION whose body '{' has not been seen yet: that
	// brace closes an operand, not a statement block, so a '/'
	// after its '}' is division (Codex P2 on #1659:
	// const n = function() {} / 2). It is gated on the keyword
	// sitting in expression position — after a function/class
	// DECLARATION a '/' starts a new statement and does open a
	// regex — and is cleared by the body's '{' or by a ';'.
	expectFuncClassBody := false

	chars := []rune(src)
	n := len(chars)

	for i := 0; i < n; i++ {
		ch := chars[i]

		if !inSingleLineComment && !inMultiLineComment && inString == 0 && !inRegex {
			if unicode.IsLetter(ch) || unicode.IsDigit(ch) || ch == '_' || ch == '$' {
				if len(curIdent) == 0 {
					identPrev = prevChar
					prevIdent = lastIdent
				}
				curIdent = append(curIdent, ch)
			} else if len(curIdent) > 0 {
				lastIdent = string(curIdent)
				lastIdentPrev = identPrev
				curIdent = nil
				if isExprKeyword(lastIdent) && lastIdentPrev != '.' {
					prevSig = 'o'
				}
				if (lastIdent == "function" || lastIdent == "class") && lastIdentPrev != '.' && isExprPositionStart(lastIdentPrev, prevIdent) {
					expectFuncClassBody = true
				}
			}
		}

		if ch == '\n' {
			line++
			inSingleLineComment = false
			if inRegex {
				// A regex literal cannot span lines; an unterminated
				// one is a genuine error, not a valid script.
				diags = append(diags, diagnose.Diagnostic{
					Kind:    diagnose.Kind("syntax-error"),
					Message: "unclosed regex literal before newline",
					Fatal:   true,
					Source:  &diagnose.Site{Line: line - 1},
				})
				inRegex = false
				inCharClass = false
				regexEscaped = false
			}
			if inString == '\'' || inString == '"' {
				// unclosed single-line string literal
				diags = append(diags, diagnose.Diagnostic{
					Kind:    diagnose.Kind("syntax-error"),
					Message: fmt.Sprintf("unclosed string literal (%c) before newline", inString),
					Fatal:   true,
					Source:  &diagnose.Site{Line: line - 1},
				})
				inString = rune(0)
			}
			continue
		}

		if inSingleLineComment {
			continue
		}

		if inMultiLineComment {
			if ch == '*' && i+1 < n && chars[i+1] == '/' {
				inMultiLineComment = false
				i++
			}
			continue
		}

		if inString != 0 {
			if isEscaped {
				isEscaped = false
				continue
			}
			if ch == '\\' {
				isEscaped = true
				continue
			}
			if ch == inString {
				inString = rune(0)
				prevSig = 'x'
			}
			continue
		}

		if inRegex {
			if regexEscaped {
				regexEscaped = false
				continue
			}
			if ch == '\\' {
				regexEscaped = true
				continue
			}
			if inCharClass {
				// Inside a character class everything is opaque —
				// delimiters, quotes, comment shapes — except the
				// closing ']'. JavaScript closes the class at the
				// first unescaped ']', even immediately after '['
				// or '[^': /[]/ is an empty class and /[^]/ a
				// negated-empty one, not classes whose first
				// character is a literal ']' (Codex P2 on #1659).
				if ch == ']' {
					inCharClass = false
				}
				continue
			}
			if ch == '[' {
				inCharClass = true
				continue
			}
			if ch == '/' {
				inRegex = false
				prevSig = 'x'
				// Consume flag letters (g, i, m, s, u, y) so the
				// completed literal stays a single operand token.
				for i+1 < n && isRegexFlag(chars[i+1]) {
					i++
				}
			}
			continue
		}

		// Comment starts
		if ch == '/' && i+1 < n {
			if chars[i+1] == '/' {
				inSingleLineComment = true
				i++
				continue
			}
			if chars[i+1] == '*' {
				inMultiLineComment = true
				i++
				continue
			}
		}

		// Regex literal vs division. A '/' opens a regex literal only
		// in expression position: at the start of the script or right
		// after an operator or opening delimiter (prevSig 0 or 'o').
		// After an operand — identifier, number, string, regex, or
		// closing delimiter — a '/' is division. This
		// previous-significant-token heuristic is the standard scanner
		// approximation. Expression keywords (return, typeof, ...),
		// control-header and statement-block closures, and postfix
		// tokens are classified explicitly below; remaining blind
		// spots are exotic shapes such as an async function
		// expression or a function statement after 'else', whose
		// keyword is not recognised as expression-position, and
		// U+2028/U+2029 appearing inside a regex literal.
		if ch == '/' {
			if prevSig == 0 || prevSig == 'o' {
				inRegex = true
				inCharClass = false
				regexEscaped = false
			} else {
				// Division: an expression (and therefore possibly a
				// regex literal) follows.
				prevSig = 'o'
			}
			continue
		}

		// String starts
		if ch == '\'' || ch == '"' || ch == '`' {
			inString = ch
			continue
		}

		// Delimiters
		switch ch {
		case '(':
			// A '(' right after a control keyword (if/for/while/
			// switch/catch/with) opens a control header: its ')'
			// returns to expression position, so a following '/'
			// opens a regex (Codex P2 on #1659: if (true) /a+/.test(..)).
			stack = append(stack, delimToken{char: ch, line: line, exprAfter: isControlKeyword(lastIdent) && lastIdentPrev != '.'})
			lastIdent = ""
			lastIdentPrev = 0
		case '{':
			// A '{' in statement position opens a block, whose '}'
			// returns to expression position ({} /a+/.test(..)); a
			// '{' in expression position opens an object literal,
			// whose '}' is an operand (({}) / 2 divides). A keyword
			// reached through member access (obj.return) is a
			// property name, not a keyword, so it does not make
			// the brace an object literal. The body of a function
			// or class expression is likewise an operand closure:
			// const n = function() {} / 2 divides.
			kwIdent := lastIdent
			if lastIdentPrev == '.' {
				kwIdent = ""
			}
			exprAfter := !isObjectBrace(prevChar, kwIdent)
			if expectFuncClassBody {
				exprAfter = false
				expectFuncClassBody = false
			}
			stack = append(stack, delimToken{char: ch, line: line, exprAfter: exprAfter})
			lastIdent = ""
			lastIdentPrev = 0
		case '[':
			stack = append(stack, delimToken{char: ch, line: line})
		case ')', '}', ']':
			if len(stack) == 0 {
				diags = append(diags, diagnose.Diagnostic{
					Kind:    diagnose.Kind("syntax-error"),
					Message: fmt.Sprintf("unmatched closing delimiter %q", ch),
					Fatal:   true,
					Source:  &diagnose.Site{Line: line},
				})
				continue
			}
			top := stack[len(stack)-1]
			stack = stack[:len(stack)-1]
			closedExprAfter = top.exprAfter
			if !matchesDelim(top.char, ch) {
				diags = append(diags, diagnose.Diagnostic{
					Kind:    diagnose.Kind("syntax-error"),
					Message: fmt.Sprintf("mismatched delimiter: opened %q on line %d, closed with %q on line %d", top.char, top.line, ch, line),
					Fatal:   true,
					Source:  &diagnose.Site{Line: line},
				})
			}
		}

		// Track the previous significant token for the regex ruling.
		wasOperand := prevSig == 'x'
		switch {
		case (ch == ')' || ch == '}') && closedExprAfter:
			prevSig = 'o'
		case ch == ')' || ch == '}' || ch == ']':
			prevSig = 'x'
		case ch == '!' && wasOperand:
			prevSig = 'x'
		case isJSWhitespace(ch):
			// Whitespace does not change the ruling.
		case unicode.IsLetter(ch) || unicode.IsDigit(ch) || ch == '_' || ch == '$':
			prevSig = 'x'
		case (ch == '+' || ch == '-') && i > 0 && chars[i-1] == ch && signAfterOperand:
			// Postfix increment/decrement: the adjacent sign pair
			// completes an operand, so a following '/' divides.
			prevSig = 'x'
		case ch == '.' && ((i > 0 && unicode.IsDigit(chars[i-1])) || (i+1 < n && unicode.IsDigit(chars[i+1]))):
			// A decimal point within a numeric token (4. / 2,
			// .5 / 2): the number is an operand, so a following
			// '/' divides.
			prevSig = 'x'
		default:
			// Operators and opening delimiters: expression position.
			prevSig = 'o'
		}
		signAfterOperand = (ch == '+' || ch == '-') && wasOperand
		if ch == ';' {
			expectFuncClassBody = false
		}
		closedExprAfter = false
		if !isJSWhitespace(ch) {
			prevChar = ch
		}
	}

	if inMultiLineComment {
		diags = append(diags, diagnose.Diagnostic{
			Kind:    diagnose.Kind("syntax-error"),
			Message: "unclosed multi-line comment (/* ... */)",
			Fatal:   true,
			Source:  &diagnose.Site{Line: line},
		})
	}
	if inRegex {
		diags = append(diags, diagnose.Diagnostic{
			Kind:    diagnose.Kind("syntax-error"),
			Message: "unclosed regex literal",
			Fatal:   true,
			Source:  &diagnose.Site{Line: line},
		})
	}
	if inString == '`' {
		diags = append(diags, diagnose.Diagnostic{
			Kind:    diagnose.Kind("syntax-error"),
			Message: "unclosed template literal (`)",
			Fatal:   true,
			Source:  &diagnose.Site{Line: line},
		})
	}

	for _, unclosed := range stack {
		diags = append(diags, diagnose.Diagnostic{
			Kind:    diagnose.Kind("syntax-error"),
			Message: fmt.Sprintf("unclosed delimiter %q opened on line %d", unclosed.char, unclosed.line),
			Fatal:   true,
			Source:  &diagnose.Site{Line: unclosed.line},
		})
	}

	return diags
}

// isExprKeyword reports whether kw is a keyword after which an
// expression (and therefore possibly a regex literal) follows:
// return /re/, typeof /re/, case /re/:, and friends.
func isExprKeyword(kw string) bool {
	switch kw {
	case "return", "throw", "case", "typeof", "void", "delete", "await", "yield", "else", "do":
		return true
	}
	return false
}

// isControlKeyword reports whether kw heads a parenthesised control
// header: if (...), for (...), while (...), switch (...), catch (...),
// with (...).
func isControlKeyword(kw string) bool {
	switch kw {
	case "if", "for", "while", "switch", "catch", "with":
		return true
	}
	return false
}

// isExprPositionStart reports whether a function/class keyword that
// started right after the significant character prev (with prevIdent
// the identifier completed before it) sits in expression position:
// after an operator or opening delimiter, or after an expression
// keyword (return function() {}). Statement position — start of
// input, or after ';', '{' or '}' — makes the keyword a declaration
// instead, whose closing brace returns to expression position.
func isExprPositionStart(prev rune, prevIdent string) bool {
	if isExprKeyword(prevIdent) {
		return true
	}
	switch prev {
	case '=', '(', ',', '[', ':', '!', '&', '|', '?', '+', '-', '*', '/', '%', '~', '^', '<', '>':
		return true
	}
	return false
}

// isJSWhitespace reports whether ch is in the ECMAScript WhiteSpace
// or LineTerminator set other than '\n' (handled separately at the
// top of the scan loop): space, tab, carriage return, vertical tab,
// form feed, no-break space, BOM/zero-width no-break space, the
// Unicode line/paragraph separators, and the remaining Unicode
// space separators. Every member must preserve the previous token's
// operand status — treating one as an operator flipped a following
// division slash into a phantom regex opener (Codex P2 on #1659:
// an operand separated from '/' by a non-breaking space).
func isJSWhitespace(ch rune) bool {
	switch ch {
	case ' ', '\t', '\r', '\v', '\f', '\u00a0', '\ufeff', '\u2028', '\u2029', '\u1680', '\u202f', '\u205f', '\u3000':
		return true
	}
	return ch >= '\u2000' && ch <= '\u200a'
}

// isObjectBrace reports whether a '{' whose previous significant
// character is prevChar (and previous completed identifier lastIdent)
// opens an object literal rather than a statement block.
func isObjectBrace(prevChar rune, lastIdent string) bool {
	switch lastIdent {
	case "return", "throw", "case", "typeof":
		return true
	}
	switch prevChar {
	case '=', '(', ',', '[', ':', '!', '&', '|', '?', '+', '-', '*', '/', '%', '~', '^':
		return true
	}
	return false
}

func isRegexFlag(ch rune) bool {
	return (ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z')
}

func matchesDelim(open, close rune) bool {
	return (open == '(' && close == ')') ||
		(open == '{' && close == '}') ||
		(open == '[' && close == ']')
}
