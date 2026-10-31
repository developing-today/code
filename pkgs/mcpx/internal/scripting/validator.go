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
// One pinned blind spot: the delimiter scanner tells regex literals from
// division with a previous-significant-token heuristic, so a regex literal
// immediately after a keyword (e.g. "return /re/") is scanned as division;
// see checkSyntaxAndDelimiters.
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
	char rune
	line int
}

func checkSyntaxAndDelimiters(src string) []diagnose.Diagnostic {
	var diags []diagnose.Diagnostic
	var stack []delimToken

	line := 1
	inSingleLineComment := false
	inMultiLineComment := false
	inString := rune(0)
	isEscaped := false
	inRegex := false
	inCharClass := false
	regexEscaped := false
	classChars := 0
	classFirst := rune(0)
	// prevSig classifies the previous significant (non-whitespace,
	// non-comment) token for the regex-vs-division ruling in the '/'
	// handling below: 0 = none yet, 'o' = operator or opening delimiter
	// (expression position: a '/' opens a regex), 'x' = operand —
	// identifier, number, string, regex, or closing delimiter (a '/'
	// there is division).
	prevSig := rune(0)

	chars := []rune(src)
	n := len(chars)

	for i := 0; i < n; i++ {
		ch := chars[i]

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
				if inCharClass {
					if classChars == 0 {
						classFirst = ch
					}
					classChars++
				}
				continue
			}
			if ch == '\\' {
				regexEscaped = true
				continue
			}
			if inCharClass {
				// Inside a character class everything is opaque —
				// delimiters, quotes, comment shapes — except the
				// closing ']'. A ']' as the first class character
				// (or right after '^') is a literal, not the close.
				if ch == ']' && classChars > 0 && !(classChars == 1 && classFirst == '^') {
					inCharClass = false
					continue
				}
				if classChars == 0 {
					classFirst = ch
				}
				classChars++
				continue
			}
			if ch == '[' {
				inCharClass = true
				classChars = 0
				classFirst = rune(0)
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
		// approximation; its known blind spot is a regex immediately
		// after a keyword such as "return", whose final identifier
		// character reads as an operand, so the pattern is scanned as
		// division and code instead of as an opaque literal.
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
		case '(', '{', '[':
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
		switch {
		case ch == ')' || ch == '}' || ch == ']':
			prevSig = 'x'
		case ch == ' ' || ch == '\t' || ch == '\r':
			// Whitespace does not change the ruling.
		case unicode.IsLetter(ch) || unicode.IsDigit(ch) || ch == '_' || ch == '$':
			prevSig = 'x'
		default:
			// Operators and opening delimiters: expression position.
			prevSig = 'o'
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

func isRegexFlag(ch rune) bool {
	return (ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z')
}

func matchesDelim(open, close rune) bool {
	return (open == '(' && close == ')') ||
		(open == '{' && close == '}') ||
		(open == '[' && close == ']')
}
