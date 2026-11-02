package scripting

import (
	"fmt"
	"strings"
	"unicode"

	"github.com/developing-today/code/pkgs/mcpx/internal/diagnose"
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
	heritage  bool
	// heritageOwnsFloor marks a heritage brace that pushed a
	// pending-body snapshot (heritageFloorStack): its closing '}'
	// restores the snapshot it displaced.
	heritageOwnsFloor bool
}

type blockCommentRange struct {
	start int
	end   int
}

// pendingBodyFrame tracks one not-yet-seen function/class
// expression body: parenDepth counts '(' opened while the frame is
// pending, stackDepth is len(stack) when the frame was pushed.
type pendingBodyFrame struct {
	parenDepth int
	stackDepth int
	arrayDepth int
}

func checkSyntaxAndDelimiters(src string) []diagnose.Diagnostic {
	var diags []diagnose.Diagnostic
	var stack []delimToken

	line := 1
	closedExprAfter := false
	inSingleLineComment := false
	inMultiLineComment := false
	blockCommentStart := -1
	lineCommentStart := -1
	var blockComments []blockCommentRange
	inString := rune(0)
	isEscaped := false
	templateIsTagged := false
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
	// curIdentBrokeLine records whether a line terminator appeared
	// in the trivia before the identifier now being accumulated
	// started (lineBreakPending at its first character; comment
	// characters never consume that flag, so a break inside an
	// intervening comment counts). The async branches below
	// consult it: async is a modifier of function only with NO
	// line terminator between the two words (Codex P2 on #3354:
	// new async<newline>function f() {} is a constructor call on
	// the identifier async plus a function declaration, by ASI).
	curIdentBrokeLine := false
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
	identPrevSig := rune(0)
	identStart := -1
	lastIdentPrev := rune(0)
	lastIdentEnd := -1
	// prevIdent is the completed identifier before the one now
	// being accumulated, captured when an identifier starts; it
	// spots expression keywords in front of function/class
	// (return function() {} / 2 divides after the body closure).
	prevIdent := ""
	// prevIdentPrev is the significant character before prevIdent
	// started (mirroring lastIdentPrev for lastIdent): it tells
	// whether prevIdent was reached through member access, in
	// which case it is a property name, not a keyword (Codex P2
	// on #3346: obj.new\nclass C {} — the property named 'new'
	// made isExprPositionStart treat the declaration as an
	// expression).
	prevIdentPrev := rune(0)
	prevIdentEnd := -1
	// prevPrevIdent is the completed identifier before prevIdent,
	// with prevPrevIdentPrev the significant character before it:
	// two levels of history are needed to spot a heritage operand
	// behind a modifier — in 'extends async function(){}' the
	// 'function' keyword's prevIdent is 'async', and only
	// prevPrevIdent still names the 'extends' that puts the async
	// function in heritage-expression position (Codex P2 on
	// #3354).
	prevPrevIdent := ""
	prevPrevIdentPrev := rune(0)
	// identDotRun / lastIdentDotRun / prevIdentDotRun mirror
	// identPrev / lastIdentPrev / prevIdentPrev for dotRun: how
	// many consecutive '.' characters immediately preceded the
	// identifier. A '.' before an identifier is member access
	// only as a single dot — after a spread '...' the identifier
	// is a keyword in its own right (Codex P2 on #3354:
	// {...new class {} / 2} — the spread's final dot made 'new'
	// read as a property name, so the class after it was taken
	// for a declaration and its body closure misread).
	identDotRun := 0
	lastIdentDotRun := 0
	prevIdentDotRun := 0
	prevPrevIdentDotRun := 0
	// funcClassFrames holds one frame per function or class
	// EXPRESSION whose body '{' has not been seen yet: that
	// brace closes an operand, not a statement block, so a '/'
	// after its '}' is division (Codex P2 on #1659:
	// const n = function() {} / 2). Frames are pushed only when
	// the keyword sits in expression position — after a
	// function/class DECLARATION a '/' starts a new statement
	// and does open a regex — and are consumed by the body's
	// '{' or cleared by a ';' in their own scope.
	// funcClassFrames is a STACK of pending function/class
	// expression bodies: a nested function/class expression inside
	// an outer one's parameter list has its own pending body, and
	// a single shared marker misclassifies the inner body closure
	// (Codex P2 on #1659: function(a = function() {} / 2) {} / 2,
	// and an inner class {} likewise). Each frame counts '('
	// opened while it is pending — a '{' inside the parameter
	// list (a default object, a destructuring pattern) is not the
	// body and must not consume the frame (function(a = {}) {} / 2)
	// — and records len(stack) when it was pushed: a ';' only
	// clears a frame from its own scope.
	var funcClassFrames []pendingBodyFrame
	// arrayDepth records the open square-bracket nesting at the current
	// token. A pending class/function body can only be consumed at the
	// bracket depth where its keyword appeared: object literals inside a
	// nested array heritage operand must not consume the class body's frame.
	arrayDepth := 0
	// inClassHeritage is set while scanning a class head before
	// its body '{': the expression after 'extends' is a heritage
	// expression in expression position (Codex P2 on #1659:
	// class C extends /a+/.constructor {}).
	inClassHeritage := false
	// heritageBraceDepth counts '{' opened inside a class heritage
	// expression itself (an object-literal heritage operand,
	// class extends {C: Object}.C {}): those braces are operands
	// of the heritage expression, not the class body, so they must
	// neither consume the pending body frame nor end the heritage
	// state (Codex P2 on #3334).
	heritageBraceDepth := 0
	// heritageFrameFloor / heritageDeclPending snapshot the pending
	// body state when the OUTERMOST heritage brace opens: a '{'
	// inside the heritage may consume only body state created after
	// that point (a function/class expression nested in the heritage
	// object), never the outer class's own pending body (Codex P2
	// on #3354: class extends {x:{C:Object}}.x.C {} / 2 — the inner
	// object ate the outer frame and the real body's '}' read as a
	// statement boundary).
	heritageFrameFloor := 0
	heritageDeclPending := false
	// heritageFloorStack / heritageDeclStack hold the snapshots
	// displaced by NESTED heritage operands: a class nested inside a
	// heritage expression has its own heritage operand, and that
	// operand's braces must protect the nested class's pending body,
	// not the outer class's (Codex P2 on #3354: class C extends
	// {x:[class extends {x:{}}.x {} / 2]}.x {} — the single outer
	// floor let the inner heritage object consume the inner class's
	// frame, so its real body's '}' read as a block closure and the
	// slash opened a phantom regex). Each floor-owning heritage
	// brace pushes the previous snapshot; its '}' restores it.
	var heritageFloorStack []int
	var heritageDeclStack []bool
	// dotRun counts consecutive '.' significant characters: a '{'
	// after a spread token '...' is the spread object operand
	// even when trivia (whitespace or a comment) intervenes
	// between the dots and the brace (Codex P2 on #3334).
	dotRun := 0
	// instAngleDepth / instActive track a speculative TypeScript
	// instantiation-expression generic list (f<number> / 2): the
	// list is recognised by the lookahead in
	// looksLikeInstantiation, and its closing '>' completes the
	// operand (Codex P2 on #3334).
	instActive := false
	instAngleDepth := 0
	// Static module declaration state (Codex P2 on #1659):
	// inModuleDecl while an import/export declaration that began
	// in statement position is being scanned; expectModuleString
	// after its 'from' keyword; stringIsModuleSpec marks the
	// string literal currently being scanned as the module
	// specifier; moduleSpecClosed records that it has closed, so
	// the next newline is a statement boundary (ASI) even though
	// the specifier string itself is an operand
	// (import "node:fs"\n/a+/.test(..)).
	inModuleDecl := false
	expectModuleString := false
	stringIsModuleSpec := false
	moduleSpecClosed := false
	// lineBreakPending records that a line terminator has been
	// scanned since the last significant token: it breaks the
	// postfix ++/-- restricted production (Codex P2 on #1659:
	// n\n++ /a+/ — the '++' is a prefix on the next statement,
	// not a postfix on n).
	lineBreakPending := false
	// expectDeclBody mirrors expectFuncClassBody for DECLARATIONS
	// (function/class in statement position, interface, enum): a
	// declaration body '{' opens a statement block whose '}'
	// returns to expression position — including after a generic
	// type-parameter list, where prevChar is '>' and the brace
	// would otherwise read as an object literal (Codex P2 on
	// #1659: class C<T> {} /a+/.test(..)).
	expectDeclBody := false
	expectDeclParenDepth := 0
	expectDeclStackDepth := 0
	// pendingStmtColon is set by a ':' that introduces a statement
	// (a switch case/default colon or a statement label) rather
	// than an object entry or a ternary branch: a '{' right after
	// it opens a statement block, not an object literal (Codex P2
	// on #1659: switch (1) { case 1: {} /a+/.test(..) }).
	pendingStmtColon := false
	// TypeScript as/satisfies assertion state: while inTypeAssert
	// a generic argument list may follow (Extract<number, number>);
	// its closing '>' completes the asserted operand, so a '/'
	// after it divides instead of opening a phantom regex
	// (Codex P2 on #1659).
	inTypeAssert := false
	typeAngleDepth := 0
	typeSawAngle := false
	// typeSawToken records that a token of the asserted type has
	// been scanned, and typeStackDepth is len(stack) when the
	// assertion began: a newline only ends the assertion once its
	// type has started AND the scan is back at the assertion's own
	// delimiter depth — a line break right after as/satisfies, or
	// inside a multi-line type, keeps the state (Codex P2 on
	// #1659: 4 as\n{ toFixed(): string } / 2).
	typeSawToken := false
	typeStackDepth := 0
	// ternaryStack records len(stack) for each unmatched '?', so a
	// ';' or ':' is scoped to its own delimiter context: a ';'
	// inside a nested callback cannot clear an outer conditional,
	// and a ':' inside a nested object cannot consume it (Codex
	// P2 on #1659: true ? (()=>{return 1;})() : {} / 2).
	var ternaryStack []int
	// prevTokenArrow records that the previous significant token
	// was the arrow '=>' (as one token), so the '{' that follows
	// is an arrow-function body block, not the object literal a
	// relational '>' would imply (Codex P2 on #1659).
	prevTokenArrow := false

	chars := []rune(src)
	n := len(chars)

	// A hashbang line (#!/...) at the very start of a script is a
	// comment, not tokens: its slashes are path separators, not
	// regex openers (Codex P2 on #1659: #!/usr/bin/env -S deno run
	// --allow-read=/tmp opened a phantom regex at the '=' slash).
	// Skip to its newline; the newline itself is scanned normally
	// so line numbers stay exact. No newline means the whole input
	// is the hashbang line and there is nothing to scan.
	scanStart := 0
	if n >= 2 && chars[0] == '#' && chars[1] == '!' {
		scanStart = n
		for j := 0; j < n; j++ {
			if isLineTerminator(chars[j]) {
				scanStart = j
				break
			}
		}
	}

	for i := scanStart; i < n; i++ {
		ch := chars[i]

		if !inSingleLineComment && !inMultiLineComment && inString == 0 && !inRegex && ch == '\\' && i+1 < n && chars[i+1] == 'u' {
			// A Unicode escape in an identifier (\u{61}, \u0061):
			// the escape is part of the identifier token, so its
			// braces are not structural delimiters (Codex P2 on
			// #3334: const \u{61} = 4; \u{61} / 2 — the escape's
			// '}' read as a block closure and the slash opened
			// a phantom regex). Consume the whole escape as one
			// identifier character (a neutral placeholder: an
			// escaped spelling never forms a keyword).
			end := -1
			if i+2 < n && chars[i+2] == '{' {
				for j := i + 3; j < n; j++ {
					if chars[j] == '}' {
						end = j
						break
					}
					if !isHexDigit(chars[j]) {
						break
					}
				}
			} else if i+5 < n && isHexDigit(chars[i+2]) && isHexDigit(chars[i+3]) && isHexDigit(chars[i+4]) && isHexDigit(chars[i+5]) {
				end = i + 5
			}
			if end >= 0 {
				if len(curIdent) == 0 {
					identStart = i
					identPrev = prevChar
					identPrevSig = prevSig
					identDotRun = dotRun
					prevPrevIdent = prevIdent
					prevPrevIdentPrev = prevIdentPrev
					prevPrevIdentDotRun = prevIdentDotRun
					prevIdent = lastIdent
					prevIdentPrev = lastIdentPrev
					prevIdentDotRun = lastIdentDotRun
					prevIdentEnd = lastIdentEnd
					curIdentBrokeLine = lineBreakPending
				}
				curIdent = append(curIdent, 'x')
				// The escape is an identifier character: it
				// carries the operand classification the main
				// loop would have given an ordinary letter.
				prevSig = 'x'
				prevChar = 'x'
				dotRun = 0
				// The escape is a significant character: it
				// consumes a pending line break like an
				// ordinary identifier character. Leaving the
				// break pending made a following ++/-- read
				// as a prefix instead of a postfix on this
				// identifier (Codex P2 on #3346:
				// \u0061++ / 2 at a line start).
				lineBreakPending = false
				i = end
				continue
			}
		}

		if !inSingleLineComment && !inMultiLineComment && inString == 0 && !inRegex {
			if isIdentChar(ch) {
				if len(curIdent) == 0 {
					identStart = i
					identPrev = prevChar
					identPrevSig = prevSig
					identDotRun = dotRun
					prevPrevIdent = prevIdent
					prevPrevIdentPrev = prevIdentPrev
					prevPrevIdentDotRun = prevIdentDotRun
					prevIdent = lastIdent
					prevIdentPrev = lastIdentPrev
					prevIdentDotRun = lastIdentDotRun
					prevIdentEnd = lastIdentEnd
					curIdentBrokeLine = lineBreakPending
				}
				curIdent = append(curIdent, ch)
			} else if len(curIdent) > 0 {
				lastIdent = string(curIdent)
				lastIdentPrev = identPrev
				lastIdentDotRun = identDotRun
				lastIdentEnd = i
				curIdent = nil
				// The next significant character after the
				// identifier, skipping whitespace and comments:
				// trivia between a keyword and a ':' must not
				// defeat the property-name guards below (Codex
				// P2 on #1659: {class : 1}).
				nextSig := nextSignificant(chars, i)
				if isExprKeyword(lastIdent) && !(lastIdentPrev == '.' && lastIdentDotRun < 3) && lastIdentPrev != '#' {
					prevSig = 'o'
				}
				if (lastIdent == "in" || lastIdent == "of" || lastIdent == "instanceof") && lastIdentPrev != '.' && lastIdentPrev != '#' && identPrevSig == 'x' {
					// Binary keyword operators: an expression follows,
					// so a '/' opens a regex literal (Codex P2 on
					// #1659: "source" in /a+/ and for (const m of
					// /a+/.exec(..))). Reached through member access
					// (obj.in) the word is a property name instead,
					// and after a non-operand (const of = 4) the word
					// is an ordinary identifier: of / 2 divides, so
					// the operator classification requires the word
					// to follow an operand (identPrevSig == 'x').
					prevSig = 'o'
				}
				if (lastIdent == "as" || lastIdent == "satisfies") && lastIdentPrev != '.' && lastIdentPrev != '#' && identPrevSig == 'x' {
					// A TypeScript assertion on an operand: a type
					// (possibly generic) follows, and the asserted
					// value stays the operand throughout.
					inTypeAssert = true
					typeAngleDepth = 0
					typeSawAngle = false
					typeSawToken = false
					typeStackDepth = len(stack)
				}
				if lastIdent == "default" && prevIdent == "export" && lastIdentPrev != '.' && lastIdentPrev != '#' {
					// export default <expression>: an expression
					// follows the keyword pair, so a '/' opens a
					// regex literal instead of dividing (Codex
					// P2 on #1659: export default /a+/.test(..)).
					// Reached through member access (obj.default)
					// the word is a property name instead.
					prevSig = 'o'
				}
				// A function/class keyword followed (after any
				// trivia) by ':', ',', '}' or ')' is an object
				// property name ({class : 1}), not a declaration:
				// it must not set pending body state that a later
				// '{' would consume (Codex P2 on #1659).
				if (lastIdent == "function" || lastIdent == "class") && lastIdentPrev != '.' && lastIdentPrev != '#' && nextSig != ':' && nextSig != ',' && nextSig != '}' && nextSig != ')' {
					if lastIdent == "class" {
						inClassHeritage = true
					}
					// A class/function keyword right after 'extends'
					// — or a 'function' keyword right after the
					// 'async' modifier that itself follows 'extends'
					// (class extends async function(){} {}: the
					// intervening 'async' must not hide the heritage
					// position, Codex P2 on #3354) — or behind the
					// 'new' of a constructor call (class extends
					// {x:[class extends new async function(){} {} / 2]}.x:
					// a 'new' callee is an expression in any context,
					// so no heritage requirement there, Codex P2 on
					// #3354) —
					// heads a heritage operand: always an expression,
					// never a declaration, so it takes a pending
					// frame even though 'extends' is not an
					// expression keyword -- otherwise its body
					// brace consumes the containing class
					// expression's frame (Codex P2 on #3354:
					// The expression position also carries across an
					// 'async' modifier in any context: for a 'function'
					// whose previous identifier is 'async' (no line
					// break between them), the position is whatever
					// precedes 'async' — so an async function in
					// assignment position inside a heritage callback
					// body (class C extends {C:()=>{const x=async
					// function(){} /2;},..}.Base {}) is an expression,
					// not a declaration (Codex P2 on #3354).
					// class C extends {x:[class extends class {}
					// {} / 2]}.x {} and the function permutation).
					asyncFunctionModifier := lastIdent == "function" && prevIdent == "async" && !curIdentBrokeLine && onlyTriviaBetween(chars, prevIdentEnd, identStart)
					if isExprPositionStart(lastIdentPrev, prevIdent, prevIdentPrev, prevIdentDotRun) || (prevIdent == "extends" && prevIdentPrev != '.' && prevIdentPrev != '#' && inClassHeritage) || (asyncFunctionModifier && prevIdentPrev != '#' && !(prevIdentPrev == '.' && prevIdentDotRun < 3) && prevPrevIdent == "extends" && prevPrevIdentPrev != '#' && !(prevPrevIdentPrev == '.' && prevPrevIdentDotRun < 3) && inClassHeritage) || (asyncFunctionModifier && prevIdentPrev != '#' && !(prevIdentPrev == '.' && prevIdentDotRun < 3) && prevPrevIdent == "new" && prevPrevIdentPrev != '#' && !(prevPrevIdentPrev == '.' && prevPrevIdentDotRun < 3)) || (asyncFunctionModifier && isExprPositionStart(prevIdentPrev, prevPrevIdent, prevPrevIdentPrev, prevPrevIdentDotRun)) {
						funcClassFrames = append(funcClassFrames, pendingBodyFrame{stackDepth: len(stack), arrayDepth: arrayDepth})
					} else {
						expectDeclBody = true
						expectDeclParenDepth = 0
						expectDeclStackDepth = len(stack)
					}
				}
				if (lastIdent == "interface" || lastIdent == "enum") && lastIdentPrev != '.' && lastIdentPrev != '#' && nextSig != ':' && nextSig != ',' && nextSig != '}' && nextSig != ')' {
					expectDeclBody = true
					expectDeclParenDepth = 0
					expectDeclStackDepth = len(stack)
				}
				if lastIdent == "extends" && lastIdentPrev != '.' && lastIdentPrev != '#' && inClassHeritage && nextSig != ':' {
					// A class heritage expression follows
					// 'extends': it is in expression position, so
					// a '/' opens a regex literal (Codex P2 on
					// #1659: class C extends /a+/.constructor {}).
					prevSig = 'o'
				}
				if (lastIdent == "import" || lastIdent == "export") && lastIdentPrev != '.' && lastIdentPrev != '#' &&
					nextSig != ':' && nextSig != ',' && nextSig != '}' && nextSig != ')' &&
					(lastIdentPrev == 0 || lastIdentPrev == ';' || lastIdentPrev == '{' || lastIdentPrev == '}') {
					// A static module declaration in statement
					// position: its module-specifier string ends
					// the declaration (Codex P2 on #1659).
					inModuleDecl = true
					expectModuleString = false
				}
				if lastIdent == "from" && lastIdentPrev != '.' && lastIdentPrev != '#' && inModuleDecl {
					expectModuleString = true
				}
			}
		}

		if isLineTerminator(ch) {
			if inString == '\'' || inString == '"' {
				if isEscaped {
					// A backslash before a line terminator inside
					// a quoted string is a line continuation: the
					// pair is string content, not the end of the
					// literal (Codex P2 on #3354: backslash + lone
					// CR regressed when CR became a terminator).
					// CRLF is one continuation: consume its LF
					// too, and count the pair as one line.
					isEscaped = false
					line++
					if ch == '\r' && i+1 < n && chars[i+1] == '\n' {
						i++
					}
					continue
				}
				if ch == '\u2028' || ch == '\u2029' {
					// Since ES2019 a line/paragraph separator is
					// ordinary content inside a quoted string,
					// not the end of the literal (Codex P2 on
					// #3354: "a<LS>b" is a valid string).
					line++
					continue
				}
			}
			// Every JavaScript line terminator (LF, CR, CRLF, LS,
			// PS) is a line break for ASI and comment termination,
			// not just LF: a lone CR between 'async' and 'function'
			// separates them the same way (Codex P2 on #3354).
			// CRLF counts once: the CR defers the increment to
			// its LF.
			if ch != '\r' || i+1 >= n || chars[i+1] != '\n' {
				line++
			}
			lineBreakPending = true
			if inSingleLineComment && lineCommentStart >= 0 {
				// Record the line comment's range alongside the
				// block comments: the numeric-token guard must
				// skip it the same way, or a digit at the end
				// of a line comment (f // 1\n<number>) is
				// mistaken for the preceding token (Codex P2
				// on #3354). The range ends at the terminator
				// that closed the comment; for an LF-closed
				// comment a trailing '\r' is the CRLF
				// terminator, not comment text.
				end := i
				if end > lineCommentStart && chars[end-1] == '\r' {
					end--
				}
				blockComments = append(blockComments, blockCommentRange{start: lineCommentStart, end: end})
				lineCommentStart = -1
			}
			inSingleLineComment = false
			if moduleSpecClosed {
				// A static import/export declaration ended at
				// its module-specifier string: ASI ends the
				// declaration at the newline, so a '/' on the
				// next line opens a regex, it is not division
				// after the specifier string (Codex P2 on
				// #1659: import "node:fs"\n/a+/.test(..)).
				prevSig = 'o'
				moduleSpecClosed = false
				inModuleDecl = false
				expectModuleString = false
				lastIdent = ""
				lastIdentPrev = 0
			}
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
			if ((lastIdent == "debugger" || lastIdent == "break" || lastIdent == "continue") ||
				((prevIdent == "break" || prevIdent == "continue") && lastIdent != "")) && lastIdentPrev != '.' && lastIdentPrev != '#' {
				// Automatic semicolon insertion ends a debugger /
				// break / continue statement at the newline: a '/'
				// on the next line opens a regex, it is not
				// division after the keyword (Codex P2 on #1659:
				// debugger\n/a+/.test(..)).
				prevSig = 'o'
				lastIdent = ""
				lastIdentPrev = 0
				// Only frames in this statement's own scope end
				// here — the same scoping the ';' expiry uses. A
				// debugger/break/continue statement nested inside
				// a pending frame's parameter default or body
				// (function(a = () => {debugger\n}) {}) must not
				// clear the outer frame (Codex P2 on #3334).
				keptFrames := funcClassFrames[:0]
				for _, fr := range funcClassFrames {
					if !(fr.parenDepth == 0 && len(stack) <= fr.stackDepth) {
						keptFrames = append(keptFrames, fr)
					}
				}
				funcClassFrames = keptFrames
			}
			if typeAngleDepth == 0 && inTypeAssert && typeSawToken && len(stack) <= typeStackDepth {
				inTypeAssert = false
				typeSawAngle = false
				typeSawToken = false
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

		// The first significant character after a line break
		// consumes it; later characters on the same line see no
		// break (postfix ++/-- classification consults this).
		brokeLine := lineBreakPending
		if !isJSWhitespace(ch) && !inSingleLineComment && !inMultiLineComment {
			// Comment characters are trivia: they must not
			// consume a pending line break, or a prefix ++/--
			// after a comment reads as a postfix on the previous
			// line's operand (Codex P2 on #3334:
			// n\n/*comment*/++ /a+/). The comment OPENER is not
			// yet flagged as a comment here; it restores the
			// pending break explicitly below.
			lineBreakPending = false
		}

		if inSingleLineComment {
			continue
		}

		if inMultiLineComment {
			if ch == '*' && i+1 < n && chars[i+1] == '/' {
				inMultiLineComment = false
				blockComments = append(blockComments, blockCommentRange{start: blockCommentStart, end: i + 2})
				blockCommentStart = -1
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
				wasTaggedTemplate := inString == '`' && templateIsTagged
				inString = rune(0)
				prevSig = 'x'
				// The closing quote is the previous significant
				// CHARACTER too: leaving prevChar at whatever
				// preceded the string (an '=' above all) made a
				// following ASI-separated '{' classify as an
				// object literal instead of a statement block
				// (Codex P2 on #1659: const x = "x"\n{} /a+/).
				prevChar = ch
				if wasTaggedTemplate {
					// A tagged template is an expression operand just like
					// a call result, so a following TypeScript
					// instantiation can bind to it (`tag`x`<T>`).
					prevChar = 'x'
				}
				templateIsTagged = false
				if stringIsModuleSpec {
					moduleSpecClosed = true
					stringIsModuleSpec = false
				}
				// The string was the keyword's operand; the keyword
				// context does not survive it (see the ';' expiry).
				lastIdent = ""
				lastIdentPrev = 0
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
				// The regex was the keyword's operand; the keyword
				// context does not survive it (see the ';' expiry).
				lastIdent = ""
				lastIdentPrev = 0
				// Consume flag letters (g, i, m, s, u, y) so the
				// completed literal stays a single operand token.
				for i+1 < n && isRegexFlag(chars[i+1]) {
					i++
				}
				// As with strings, the completed literal is the
				// previous significant character for a following
				// '{' classification: with flags it is the last
				// flag letter; without, an operand sentinel —
				// the '/' itself is an operator character and
				// would misclassify the brace (Codex P2 on
				// #1659: const x = /x/\n{} /a+/). The sentinel
				// 'x' is an identifier character, which every
				// prevChar consumer reads as operand position.
				if i < n && isRegexFlag(chars[i]) && chars[i] != '/' {
					prevChar = chars[i]
				} else {
					prevChar = 'x'
				}
			}
			continue
		}

		// Comment starts
		if ch == '/' && i+1 < n {
			if chars[i+1] == '/' {
				inSingleLineComment = true
				lineCommentStart = i
				if brokeLine {
					lineBreakPending = true
				}
				i++
				continue
			}
			if chars[i+1] == '*' {
				inMultiLineComment = true
				blockCommentStart = i
				if brokeLine {
					lineBreakPending = true
				}
				i++
				continue
			}
		}

		if inTypeAssert && !isJSWhitespace(ch) {
			typeSawToken = true
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
		// Instantiation expressions are recognised only for
		// primitive or capitalised type names (see
		// looksLikeInstantiation): f<lowercase> / 2 remains a
		// pinned blind spot, since it is indistinguishable from
		// a relational chain by local inspection.
		if ch == '/' {
			// Slash is a significant token (division or a regex
			// literal), so an earlier spread's dot run cannot leak
			// through it into a later `.new` property.
			dotRun = 0
			if inTypeAssert && typeAngleDepth == 0 {
				// The assertion carried no generic list
				// (x as number / 2): it is already complete, and
				// this '/' is judged by the normal ruling.
				inTypeAssert = false
				typeSawAngle = false
			}
			if prevSig == 0 || prevSig == 'o' {
				inRegex = true
				inCharClass = false
				regexEscaped = false
			} else {
				// Division: an expression (and therefore possibly a
				// regex literal) follows. The division operator is
				// also the previous significant CHARACTER: leaving
				// prevChar at the dividend made a following '{'
				// classify by the dividend instead of the '/'
				// (Codex P2 on #1659: 4 / {} / 2 — the object
				// brace after division was read as a block).
				prevSig = 'o'
				prevChar = ch
			}
			continue
		}

		// String starts
		if ch == '\'' || ch == '"' || ch == '`' {
			templateIsTagged = ch == '`' && prevSig == 'x'
			inString = ch
			// Literal tokens interrupt a preceding spread token.
			dotRun = 0
			// A string directly after 'import' (a side-effect
			// import) or after a module declaration's 'from' is
			// the module specifier: the declaration ends with
			// it (Codex P2 on #1659).
			stringIsModuleSpec = inModuleDecl && (expectModuleString || lastIdent == "import")
			continue
		}

		// Delimiters
		switch ch {
		case '(':
			for fi := range funcClassFrames {
				funcClassFrames[fi].parenDepth++
			}
			if expectDeclBody {
				expectDeclParenDepth++
			}
			// A '(' right after a control keyword (if/for/while/
			// switch/catch/with) opens a control header: its ')'
			// returns to expression position, so a following '/'
			// opens a regex (Codex P2 on #1659: if (true) /a+/.test(..)).
			// A 'for await (' header reaches this point with
			// lastIdent == "await" (prevIdent == "for"): it is a
			// control header too, whose ')' returns to expression
			// position (Codex P2 on #1659: for await (const x of
			// [1]) /a+/.test(..)).
			isControl := isControlKeyword(lastIdent) || (lastIdent == "await" && prevIdent == "for")
			stack = append(stack, delimToken{char: ch, line: line, exprAfter: isControl && lastIdentPrev != '.' && lastIdentPrev != '#'})
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
			if lastIdentPrev == '.' || lastIdentPrev == '#' {
				kwIdent = ""
			}
			exprAfter := !isObjectBrace(prevChar, kwIdent)
			if prevTokenArrow {
				// An arrow-function body is a block, not an
				// object literal: its '}' ends the function
				// expression, so a '/' starting the next
				// (ASI-separated) statement opens a regex
				// (Codex P2 on #1659). A relational '>' keeps
				// the object reading (1 > {} / 2 divides).
				exprAfter = true
			}
			if pendingStmtColon {
				exprAfter = true
				pendingStmtColon = false
			}
			if inTypeAssert {
				// A '{' opening an as/satisfies TYPE is a type
				// literal: its '}' completes the asserted operand,
				// so a '/' after it divides, not opens a regex
				// (Codex P2 on #1659: 4 as { toFixed(): string } / 2).
				exprAfter = false
			}
			if dotRun >= 3 || (i >= 3 && chars[i-1] == '.' && chars[i-2] == '.' && chars[i-3] == '.') {
				// A '{' right after a spread token is the spread
				// object operand, not a statement block: its '}'
				// is an operand, so a following '/' divides
				// (Codex P2 on #1659: {...{} / 2}). dotRun counts
				// significant '.' characters, so trivia between
				// the spread token and the brace does not defeat
				// it (Codex P2 on #3334: {... /*c*/ {} / 2}).
				exprAfter = false
			}
			// A '{' inside a class heritage expression at
			// expression position (right after 'extends' or an
			// operator), or nested inside such a brace, is a
			// heritage operand — an object literal — not the
			// class body: it must not consume the pending body
			// frame (Codex P2 on #3334: class extends
			// {C: Object}.C {} / 2 — the heritage object ate
			// the frame and the body's '}' read as a block
			// closure, opening a phantom regex at the slash).
			// After a completed operand closer ('>', ')', ']')
			// a '{' cannot start another heritage operand —
			// class C<T> {} and class C extends mixin() {} are
			// bodies, not heritage objects.
			// A pending function/class expression body (or a
			// declaration body) that opens INSIDE an already
			// open heritage brace is a body, not another
			// heritage operand, and braces nested inside it
			// classify by the ordinary rules (Codex P2 on
			// #3346: class C extends {C: function(){ {}
			// /[}]/.test('}'); }}.C {} — the heritage depth
			// forced the function's inner statement block to
			// read as an object literal, so the following
			// regex read as division). An arrow body is a
			// body too, never a heritage operand.
			// Both frame-consumption checks below also require
			// the brace to sit at the pending frame's own
			// delimiter depth (len(stack) <= frame.stackDepth,
			// the same scoping the ';' expiry uses): a '{' nested
			// inside a body opened after the frame was pushed --
			// a statement block inside a heritage operand's
			// function body, a method body inside a heritage
			// operand's class body -- belongs to that inner body,
			// never to the pending outer frame (Codex P2 on
			// #3354: class C extends {x:[class extends
			// function(){ {} } {} /2]}.x {} and the class-method
			// permutation {x:[class extends class {m(){}} {} /2]}
			// -- the inner block/method brace consumed the
			// containing class's pending frame, its real body
			// then closed as a statement boundary and the slash
			// opened a phantom regex).
			topHeritageBrace := len(stack) > 0 && stack[len(stack)-1].char == '{' && stack[len(stack)-1].heritage
			insideHeritageBrace := topHeritageBrace || heritageBraceDepth > 0
			// A '{' immediately after 'extends' opens that class's
			// heritage operand, not its body — even when the class
			// is itself nested inside an outer heritage brace
			// (Codex P2 on #3354: {x:[class extends {C:Object}.C
			// {} / 2]} — frameBodyBrace consumed the inner class's
			// frame at its heritage brace, so its real body's '}'
			// read as a block closure and the slash opened a
			// phantom regex).
			// A heritage operand can also open with the 'new'
			// keyword: in class C extends new {C:Object}.C {}
			// the brace after 'new' is the constructor operand,
			// not the class body (Codex P2 on #3354: nested as
			// {x:[class extends new {C:Object}.C {} / 2]} the
			// brace ate the inner class's frame and the real
			// body's '}' opened a phantom regex at the slash).
			// 'new' reached through member access (obj.new) is a
			// property name, not the keyword.
			heritageKeyword := lastIdent == "extends" || (lastIdent == "new" && lastIdentPrev != '.' && lastIdentPrev != '#')
			frameBodyBrace := insideHeritageBrace && len(funcClassFrames) > heritageFrameFloor && funcClassFrames[len(funcClassFrames)-1].parenDepth == 0 && funcClassFrames[len(funcClassFrames)-1].arrayDepth == arrayDepth && len(stack) <= funcClassFrames[len(funcClassFrames)-1].stackDepth && !heritageKeyword
			declBodyBrace := insideHeritageBrace && expectDeclBody && !heritageDeclPending && expectDeclParenDepth == 0
			heritageExprBrace := inClassHeritage && !frameBodyBrace && !declBodyBrace && !prevTokenArrow &&
				(topHeritageBrace ||
					(heritageBraceDepth == 0 && prevSig == 'o' && prevChar != '>' && prevChar != ')' && prevChar != ']') ||
					(heritageBraceDepth > 0 && prevSig == 'o' && heritageKeyword))
			heritageOwnsFloor := false
			if heritageExprBrace {
				// A heritage brace owns a floor snapshot when it
				// starts a heritage operand: the outermost one
				// (depth 0), or a nested class's operand opened
				// right after 'extends' inside an outer heritage
				// expression. Plain braces nested inside an
				// operand (object values) share its floor.
				if heritageBraceDepth == 0 || (prevSig == 'o' && heritageKeyword) {
					heritageFloorStack = append(heritageFloorStack, heritageFrameFloor)
					heritageDeclStack = append(heritageDeclStack, heritageDeclPending)
					heritageFrameFloor = len(funcClassFrames)
					heritageDeclPending = expectDeclBody
					heritageOwnsFloor = true
				}
				heritageBraceDepth++
				exprAfter = false
			}
			if !heritageExprBrace && len(funcClassFrames) > 0 && funcClassFrames[len(funcClassFrames)-1].parenDepth == 0 && funcClassFrames[len(funcClassFrames)-1].arrayDepth == arrayDepth && len(stack) <= funcClassFrames[len(funcClassFrames)-1].stackDepth && (!insideHeritageBrace || len(funcClassFrames) > heritageFrameFloor) {
				exprAfter = false
				funcClassFrames = funcClassFrames[:len(funcClassFrames)-1]
			}
			if !heritageExprBrace && !insideHeritageBrace {
				// A body brace ends the heritage only when it is
				// not itself nested inside a heritage brace: an
				// arrow/function body inside the heritage object
				// leaves the heritage state for the tokens after
				// it (Codex P2 on #3354: {C:()=>{const a={};}
				// — resetting here let the inner {} consume the
				// outer class frame).
				inClassHeritage = false
				heritageBraceDepth = 0
			}
			if !heritageExprBrace && expectDeclBody && expectDeclParenDepth == 0 && (!insideHeritageBrace || !heritageDeclPending) {
				exprAfter = true
				expectDeclBody = false
			}
			stack = append(stack, delimToken{char: ch, line: line, exprAfter: exprAfter, heritage: heritageExprBrace, heritageOwnsFloor: heritageOwnsFloor})
			lastIdent = ""
			lastIdentPrev = 0
		case '[':
			stack = append(stack, delimToken{char: ch, line: line})
			arrayDepth++
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
			if top.char == '[' && arrayDepth > 0 {
				arrayDepth--
			}
			if ch == '}' && top.heritage && heritageBraceDepth > 0 {
				heritageBraceDepth--
				if top.heritageOwnsFloor && len(heritageFloorStack) > 0 {
					heritageFrameFloor = heritageFloorStack[len(heritageFloorStack)-1]
					heritageFloorStack = heritageFloorStack[:len(heritageFloorStack)-1]
					heritageDeclPending = heritageDeclStack[len(heritageDeclStack)-1]
					heritageDeclStack = heritageDeclStack[:len(heritageDeclStack)-1]
				}
			}
			if top.char == '(' {
				for fi := range funcClassFrames {
					if funcClassFrames[fi].parenDepth > 0 {
						funcClassFrames[fi].parenDepth--
					}
				}
			}
			if top.char == '(' && expectDeclBody && expectDeclParenDepth > 0 {
				expectDeclParenDepth--
			}
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
		if ch != '{' && !isJSWhitespace(ch) {
			// A pending statement colon is spent by the first
			// significant token after it if that token is not the
			// '{' it was classifying (handled in the delimiter
			// switch above).
			pendingStmtColon = false
		}
		switch {
		case (ch == ')' || ch == '}') && closedExprAfter:
			prevSig = 'o'
			if typeAngleDepth == 0 {
				inTypeAssert = false
				typeSawAngle = false
			}
		case ch == ')' || ch == '}' || ch == ']':
			prevSig = 'x'
			if typeAngleDepth == 0 {
				inTypeAssert = false
				typeSawAngle = false
			}
		case ch == '?':
			// Optional chaining ('?.', unless a digit follows the
			// '.' — a ? .5 : b is a ternary with a fractional
			// branch) and nullish coalescing ('??') are not
			// conditional-expression openers: counting them makes
			// a later statement colon look like a ternary branch
			// (Codex P2 on #1659: switch (obj?.x) { case 1: ... }).
			if !(i+1 < n && (chars[i+1] == '?' || (chars[i+1] == '.' && !(i+2 < n && unicode.IsDigit(chars[i+2]))))) && !(i > 0 && chars[i-1] == '?') {
				// The second '?' of '??' is excluded by the
				// previous-character check: when it is scanned,
				// the lookahead above no longer sees the pair
				// (Codex P2 on #1659: switch (null ?? 1) {...}).
				ternaryStack = append(ternaryStack, len(stack))
			}
			prevSig = 'o'
		case ch == ':':
			if len(ternaryStack) > 0 && ternaryStack[len(ternaryStack)-1] == len(stack) {
				ternaryStack = ternaryStack[:len(ternaryStack)-1]
			} else if isStatementColon(stack, lastIdent, lastIdentPrev, prevChar) {
				pendingStmtColon = true
			}
			prevSig = 'o'
		case ch == '<' && instActive:
			instAngleDepth++
			prevSig = 'x'
		case ch == '>' && instActive:
			instAngleDepth--
			if instAngleDepth <= 0 {
				instAngleDepth = 0
				instActive = false
			}
			prevSig = 'x'
		case ch == '<' && !inTypeAssert && prevSig == 'x' && !prevTokenIsNumeric(chars, i, blockComments) && prevChar != '\'' && prevChar != '"' && prevChar != '`' && looksLikeInstantiation(chars, i):
			// A TypeScript instantiation expression
			// (f<number> / 2): the generic list's closing '>'
			// completes the operand, so the slash divides
			// (Codex P2 on #3334). Recognition is deliberately
			// narrow (see looksLikeInstantiation) so ordinary
			// relational chains keep the operator reading.
			instActive = true
			instAngleDepth = 1
			prevSig = 'x'
		case ch == '<' && inTypeAssert && typeAngleDepth == 0 && !typeSawAngle && (prevChar == ']' || isSimpleTypeName(lastIdent)):
			// A completed simple type (4 as number < /a+/) or a
			// bracketed type (x as T[] < y) cannot take a generic
			// argument list, so this '<' is relational and the
			// assertion context ends before it is classified
			// (Codex P2 on #1659).
			inTypeAssert = false
			prevSig = 'o'
		case ch == '<' && inTypeAssert:
			typeAngleDepth++
			typeSawAngle = true
			prevSig = 'x'
		case ch == '>' && inTypeAssert && typeSawAngle:
			typeAngleDepth--
			if typeAngleDepth <= 0 {
				typeAngleDepth = 0
				inTypeAssert = false
				typeSawAngle = false
			}
			prevSig = 'x'
		case ch == '!' && wasOperand:
			prevSig = 'x'
		case ch == '#' && i+1 < n && isIdentChar(chars[i+1]):
			// A '#' directly before an identifier is a private
			// name (#return): the name that follows is a property
			// name, never a keyword — every keyword guard above
			// therefore also excludes lastIdentPrev == '#' — and
			// the pair is an operand fragment (Codex P2 on
			// #3334: this.#return / 2 read the slash as a regex
			// opener after the phantom return keyword).
			prevSig = 'x'
		case isJSWhitespace(ch):
			// Whitespace does not change the ruling.
		case isIdentChar(ch):
			prevSig = 'x'
		case (ch == '+' || ch == '-') && i > 0 && chars[i-1] == ch && signAfterOperand && !brokeLine:
			// Postfix increment/decrement: the adjacent sign pair
			// completes an operand, so a following '/' divides.
			// A line break before the pair makes it a prefix on
			// the next statement instead (restricted production).
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
		// A line break before the sign breaks the postfix
		// production, so the sign does not arm a postfix pair.
		signAfterOperand = (ch == '+' || ch == '-') && wasOperand && !brokeLine
		if ch == ';' {
			// Only a ';' in a pending frame's own scope clears
			// it: a ';' inside a nested parameter default
			// (function(a = () => { return 1; })) belongs to the
			// nested scope and must not clear the outer frame
			// (Codex P2 on #1659).
			kept := funcClassFrames[:0]
			for _, fr := range funcClassFrames {
				if !(fr.parenDepth == 0 && len(stack) <= fr.stackDepth) {
					kept = append(kept, fr)
				}
			}
			funcClassFrames = kept
			inClassHeritage = false
			inModuleDecl = false
			expectModuleString = false
			stringIsModuleSpec = false
			moduleSpecClosed = false
			if expectDeclParenDepth == 0 && len(stack) <= expectDeclStackDepth {
				expectDeclBody = false
				expectDeclParenDepth = 0
			}
			// Only ternaries opened at this ';' own delimiter
			// depth or deeper end with the statement; an outer
			// conditional whose branch contains this nested
			// statement keeps its pending '?' (Codex P2 on
			// #1659).
			for len(ternaryStack) > 0 && ternaryStack[len(ternaryStack)-1] >= len(stack) {
				ternaryStack = ternaryStack[:len(ternaryStack)-1]
			}
			inTypeAssert = false
			typeAngleDepth = 0
			typeSawAngle = false
			typeSawToken = false
			instActive = false
			instAngleDepth = 0
			// A statement boundary expires any keyword context: a
			// '{' after ';' must not be classified by a keyword an
			// earlier statement ended with (Codex P2 on #1659:
			// typeof ""; {} /a+/.test(..) — the stale typeof made
			// the block an object literal).
			lastIdent = ""
			lastIdentPrev = 0
		}
		closedExprAfter = false
		if !isJSWhitespace(ch) {
			// The arrow is a single token: a '>' immediately
			// after '=' in the source is '=>'; any other
			// significant character ends the arrow context.
			prevTokenArrow = ch == '>' && i > 0 && chars[i-1] == '='
			prevChar = ch
			if ch == '.' {
				dotRun++
			} else {
				dotRun = 0
			}
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
// return /re/, typeof /re/, case /re/:, and friends. "new" is
// included: its operand is an expression, so new /a+/ constructs
// the regex (Codex P2 on #3334: new stayed classified as an
// operand and the slash was read as division).
func isExprKeyword(kw string) bool {
	switch kw {
	case "return", "throw", "case", "typeof", "void", "delete", "await", "yield", "else", "do", "new":
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
// the identifier completed before it, and prevIdentPrev the
// significant character before prevIdent — a '.' or '#' there makes
// prevIdent a property name, not an expression keyword) sits in
// expression position:
// after an operator or opening delimiter, or after an expression
// keyword (return function() {}). Statement position — start of
// input, or after ';', '{' or '}' — makes the keyword a declaration
// instead, whose closing brace returns to expression position.
func isExprPositionStart(prev rune, prevIdent string, prevIdentPrev rune, prevIdentDotRun int) bool {
	if !(prevIdentPrev == '.' && prevIdentDotRun < 3) && prevIdentPrev != '#' && isExprKeyword(prevIdent) {
		return true
	}
	switch prev {
	case '=', '(', ',', '[', ':', '!', '&', '|', '?', '+', '-', '*', '/', '%', '~', '^', '<', '>':
		return true
	}
	return false
}

// isIdentChar reports whether ch can appear in a JavaScript identifier
// as this scanner tracks it: letters, digits, '_' and '$', plus the
// remaining IdentifierPart characters — combining marks (Mn, Mc),
// connector punctuation (Pc), letter numbers (Nl, e.g. U+2160 —
// ECMAScript ID_Start/ID_Continue include them, but Go's IsLetter and
// IsDigit do not; leaving them out split the identifier Ⅰ1 into the
// number 1 in the numeric-token scan, Codex P2 on #3354), and the
// zero-width (non-)joiners U+200C and
// U+200D. An identifier ending in one of those is still an operand, so a
// '/' after it divides (Codex P2 on #1659: an identifier ending in
// U+0301 fell out of the accumulation and the classification alike).
func isIdentChar(ch rune) bool {
	return unicode.IsLetter(ch) || unicode.IsDigit(ch) ||
		ch == '_' || ch == '$' ||
		unicode.In(ch, unicode.Mn, unicode.Mc, unicode.Pc, unicode.Nl) ||
		ch == '\u200c' || ch == '\u200d'
}

// isLineTerminator reports whether ch is a JavaScript line
// terminator: LF, CR, U+2028 (line separator), or U+2029
// (paragraph separator). CRLF is a single terminator made of the
// first two. The scan loop's terminator branch handles all four;
// treating only LF as a break left CR/LS/PS invisible to ASI
// classification (Codex P2 on #3354).
func isLineTerminator(ch rune) bool {
	return ch == '\n' || ch == '\r' || ch == '\u2028' || ch == '\u2029'
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
	// The expression keywords of isExprKeyword, minus else/do:
	// their braces open statement blocks, not object literals
	// (Codex P2 on #1659: void {} / 2 divided after a phantom
	// block closure).
	switch lastIdent {
	case "return", "throw", "case", "typeof", "void", "delete", "await", "yield":
		return true
	}
	switch prevChar {
	case '=', '(', ',', '[', ':', '!', '&', '|', '?', '+', '-', '*', '/', '%', '~', '^', '<', '>':
		return true
	}
	return false
}

// isStatementColon reports whether a ':' at the current scan point
// introduces a statement — a switch case/default colon or a statement
// label — rather than an object entry or a ternary branch. The caller
// has already excluded the ternary case (an unmatched '?' pending).
// A '{' after a statement colon opens a statement block, whose '}'
// returns to expression position.
func isStatementColon(stack []delimToken, lastIdent string, lastIdentPrev, prevChar rune) bool {
	if lastIdent == "case" {
		return true
	}
	if len(stack) > 0 {
		top := stack[len(stack)-1]
		// Inside a statement block (a switch body above all) a
		// colon ends a case/default clause or a label.
		return top.char == '{' && top.exprAfter
	}
	// A label at top level: an identifier that started at a
	// statement boundary, directly followed by the colon.
	if lastIdent != "" && isIdentChar(prevChar) {
		switch lastIdentPrev {
		case 0, ';', '{', '}':
			return true
		}
	}
	return false
}

// isSimpleTypeName reports whether name is a TypeScript type that
// cannot take generic arguments, so a '<' after it in an as/satisfies
// assertion is a relational operator, not a type-argument opener.
func isSimpleTypeName(name string) bool {
	switch name {
	case "number", "string", "boolean", "bigint", "symbol", "any", "unknown", "never", "void", "object", "undefined", "null", "this":
		return true
	}
	return false
}

// onlyTriviaBetween reports whether the source between two identifier
// boundaries contains only whitespace or comments. The async modifier
// classification uses it to distinguish `async function` from an unrelated
// earlier identifier named async, even when comments separate the tokens.
func onlyTriviaBetween(chars []rune, start, end int) bool {
	if start < 0 || end < start || end > len(chars) {
		return false
	}
	for i := start; i < end; {
		ch := chars[i]
		if isJSWhitespace(ch) || ch == '\n' {
			i++
			continue
		}
		if ch == '/' && i+1 < end {
			switch chars[i+1] {
			case '/':
				i += 2
				for i < end && !isLineTerminator(chars[i]) {
					i++
				}
				continue
			case '*':
				i += 2
				for i+1 < end && !(chars[i] == '*' && chars[i+1] == '/') {
					i++
				}
				if i+1 >= end {
					return false
				}
				i += 2
				continue
			}
		}
		return false
	}
	return true
}

// nextSignificant returns the first significant (non-whitespace,
// non-comment) character at or after index from, or 0 if there is
// none. It lets keyword classification look past trivia — spaces
// or comments between an identifier and a following ':' must not
// change the ruling (Codex P2 on #1659: {class : 1}).
func nextSignificant(chars []rune, from int) rune {
	n := len(chars)
	for j := from; j < n; j++ {
		c := chars[j]
		if isJSWhitespace(c) || c == '\n' {
			continue
		}
		if c == '/' && j+1 < n && chars[j+1] == '/' {
			for j < n && !isLineTerminator(chars[j]) {
				j++
			}
			continue
		}
		if c == '/' && j+1 < n && chars[j+1] == '*' {
			j += 2
			for j+1 < n && !(chars[j] == '*' && chars[j+1] == '/') {
				j++
			}
			j++
			continue
		}
		return c
	}
	return 0
}

// isHexDigit reports whether ch is an ASCII hex digit (for
// \uXXXX and \u{...} identifier escapes).
func isHexDigit(ch rune) bool {
	return (ch >= '0' && ch <= '9') || (ch >= 'a' && ch <= 'f') || (ch >= 'A' && ch <= 'F')
}

// prevTokenIsNumeric reports whether the token immediately before
// offset lt is a numeric literal: it walks back over identifier
// characters and checks the token's FIRST character, so an
// identifier that merely ends in a digit (f1) is not mistaken for
// a number (Codex P2 on #3354: f1<number> / 2 lost instantiation
// recognition to a guard that looked only at the final character,
// and the relational '>' left the slash to open a phantom regex).
// isIdentExceptionRune reports whether ch is one of the ECMAScript
// Unicode identifier exceptions: a rune outside the general
// ID_Start/ID_Continue categories that the spec still allows inside
// an identifier — the ID_Continue exceptions U+00B7, U+0387,
// U+1369–U+1371 and U+19DA (plus U+30FB/U+FF65, letter-adjacent in
// Unicode but listed for completeness) and the Other_ID_Start
// exceptions U+2118, U+212E, U+309B and U+309C. isIdentChar does not
// cover these, so a scan that stops at one splits the identifier
// (Codex P2 on #3354: f\u00b71 and \u21181 read as the number 1).
func isIdentExceptionRune(ch rune) bool {
	switch ch {
	case '\u00b7', '\u0387', '\u19da', '\u2118', '\u212e', '\u309b', '\u309c', '\u30fb', '\uff65':
		return true
	}
	return ch >= '\u1369' && ch <= '\u1371'
}

func prevTokenIsNumeric(chars []rune, lt int, comments []blockCommentRange) bool {
	end := lt
	// Skip trailing trivia — whitespace, line terminators and
	// comments (block and line ranges are both recorded) — the way the scanner's prevChar already
	// does: in "1 < x" the character before '<' is a space, but
	// the token is "1", and a newline before '<' does not change
	// the token either ("1\n< x").
	for end > 0 {
		for end > 0 && (isJSWhitespace(chars[end-1]) || chars[end-1] == '\n') {
			end--
		}
		skippedComment := false
		// Comment ranges are recorded in scan order, so their
		// ends are sorted: binary-search for the range ending
		// exactly at end instead of scanning the whole history
		// for every '<' -- the linear scan made validation
		// quadratic in the number of block comments (Codex P2
		// on #3354: 60k comments + comparisons, ~25x slower).
		lo, hi := 0, len(comments)
		for lo < hi {
			mid := int(uint(lo+hi) >> 1)
			if comments[mid].end < end {
				lo = mid + 1
			} else {
				hi = mid
			}
		}
		// The match is containment, not just an exact end: the
		// whitespace strip above can move end INSIDE a comment
		// whose text ends in spaces or tabs (f // 1   \n<number>),
		// and an exact-end lookup then misses the range and reads
		// the comment's trailing digit as the previous token
		// (Codex P2 on #3354). Ranges are disjoint and sorted, so
		// the first range ending at or after end is the only one
		// that can contain it.
		if lo < len(comments) && comments[lo].start < end && end <= comments[lo].end {
			end = comments[lo].start
			skippedComment = true
		}
		if skippedComment {
			continue
		}
		break
	}
	start := end
	// A numeric literal may end in a decimal point (1. < x): the
	// trailing '.' belongs to the number token when a digit
	// precedes it, so step over it before walking the token —
	// otherwise the walk stops at the '.' and the number is
	// missed (CodeRabbit finding on #3354).
	if start > 1 && chars[start-1] == '.' && unicode.IsDigit(chars[start-2]) {
		start--
	}
	for start > 0 {
		if isIdentChar(chars[start-1]) || isIdentExceptionRune(chars[start-1]) {
			start--
			continue
		}
		// A braced Unicode escape (\u{61}) belongs to the token
		// even though its '}' is not an identifier character:
		// jump over the whole escape and keep walking, so
		// f\u{61}1 reads as one identifier, not as the number 1
		// (Codex P2 on #3354).
		if chars[start-1] == '}' {
			open := start - 1
			for open > 0 && chars[open] != '{' {
				open--
			}
			if open >= 2 && chars[open] == '{' && chars[open-1] == 'u' && chars[open-2] == '\\' {
				start = open - 2
				continue
			}
		}
		break
	}
	return start < end && unicode.IsDigit(chars[start])
}

// looksLikeInstantiation reports whether the '<' at index lt opens
// a TypeScript instantiation-expression generic list whose operand
// classification matters: the list must balance using only
// type-shaped tokens, must contain at least one recognisable type
// name (a primitive type or a capitalised identifier — the shapes
// generic arguments actually take), and the token after the
// closing '>' must be one that follows an operand (a '/', '(',
// '.', operator, or delimiter) rather than another operand, which
// would make the '<' a relational operator instead
// (a < Foo > b is a comparison chain, f<Foo> / 2 is not).
func looksLikeInstantiation(chars []rune, lt int) bool {
	n := len(chars)
	depth := 0
	sawTypeName := false
	for j := lt; j < n; j++ {
		c := chars[j]
		switch {
		case isIdentChar(c):
			start := j
			for j < n && isIdentChar(chars[j]) {
				j++
			}
			name := string(chars[start:j])
			j--
			if isSimpleTypeName(name) || name == "keyof" || name == "typeof" || name == "readonly" {
				sawTypeName = true
			} else if len(name) > 0 {
				if r := []rune(name)[0]; unicode.IsUpper(r) {
					sawTypeName = true
				}
			}
		case c == '<':
			depth++
		case c == '>':
			depth--
			if depth == 0 {
				if !sawTypeName {
					return false
				}
				switch nextSignificant(chars, j+1) {
				case '/', '(', '.', '[', ';', ',', ')', ']', '}', '?', ':', '+', '-', '*', '%', '=', '!', '&', '|', '<', '>', 0:
					return true
				}
				return false
			}
		case isJSWhitespace(c) || c == '\n' || c == ',' || c == '.' || c == '[' || c == ']' ||
			c == '{' || c == '}' || c == '(' || c == ')' || c == '|' || c == '&' || c == '?' ||
			c == ':' || c == ';' || c == '-' || c == '#':
			// Type-shaped punctuation.
		case c == '=' && j+1 < n && chars[j+1] == '>':
			j++
		case c == '\'' || c == '"' || c == '`':
			// A literal type: skip to its closing quote.
			for j++; j < n && chars[j] != c; j++ {
				if chars[j] == '\\' {
					j++
				}
			}
		default:
			return false
		}
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
