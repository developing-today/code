package diagnose

import (
	"fmt"
	"sort"
	"strings"
)

// shapeFit is rung 1's second half (#1927): the deterministic
// argument-shape fit for substitutions the near-miss name matcher
// cannot see. search -> query is edit distance 5 and olderThan ->
// olderThanDays is 4, so no name threshold finds either; what gives
// them away is shape. The supplied argument names satisfy exactly one
// other tool's schema in the namespace, or a supplied name is a
// prefix of exactly one declared parameter of the called tool.
//
// The fit is only offered when it is unique. Two candidate tools, or
// two candidate parameters, means the deterministic answer is "no
// suggestion" — a guessed rewrite is worse than the raw error, and
// the generic diagnostic still runs. The fit is also only considered
// when at least one supplied name is undeclared on the called tool:
// a call whose names all fit the called tool failed for some other
// reason, and a substitution suggestion would be noise.
func shapeFit(in CallErrorInput, tool Tool, sent map[string]bool, cat Catalog) (Diagnostic, bool) {
	var extra []string
	for name := range sent {
		if _, ok := tool.Shape.Props[name]; !ok {
			extra = append(extra, name)
		}
	}
	if len(sent) == 0 || len(extra) == 0 {
		return Diagnostic{}, false
	}
	sort.Strings(extra)

	// Another tool in the namespace whose schema the sent names fit:
	// every sent name is declared there, and every name it requires
	// was sent.
	var fits []Tool
	for _, t := range cat.Tools {
		if t.Namespace != tool.Namespace || t.Path() == tool.Path() {
			continue
		}
		if len(t.Shape.Props) == 0 {
			continue
		}
		fit := true
		for name := range sent {
			if _, has := t.Shape.Props[name]; !has {
				fit = false
				break
			}
		}
		if fit {
			for _, req := range t.Shape.Required {
				if !sent[req] {
					fit = false
					break
				}
			}
		}
		if fit {
			fits = append(fits, t)
		}
	}
	switch {
	case len(fits) == 1:
		t := fits[0]
		return Diagnostic{
			Kind: KindInvalidParams, Tool: tool.Path(), Fatal: true,
			Message: fmt.Sprintf("the arguments fit %s, not %s: the names sent (%s) are exactly the arguments %s takes",
				t.Path(), tool.Path(), strings.Join(sortedSent(sent), ", "), t.Path()),
			Changed: cat.change(t.Path(), ""),
			Source:  in.Source,
			Fix:     fixWith(Call{}, t, t.Shape.Required),
		}, true
	case len(fits) > 1:
		// Ambiguous: suggest nothing.
		return Diagnostic{}, false
	}

	// A supplied name that is a prefix of exactly one declared
	// parameter of the called tool (olderThan -> olderThanDays).
	var from, to string
	unique := true
	for _, name := range extra {
		var cands []string
		for prop := range tool.Shape.Props {
			if prop != name && strings.HasPrefix(prop, name) {
				cands = append(cands, prop)
			}
		}
		if len(cands) == 0 {
			continue
		}
		if len(cands) > 1 || from != "" {
			unique = false
			break
		}
		from, to = name, cands[0]
	}
	if !unique || from == "" {
		return Diagnostic{}, false
	}
	return Diagnostic{
		Kind: KindInvalidParams, Tool: tool.Path(), Field: to, Fatal: true,
		Message: fmt.Sprintf("%s is not an argument of %s; the one declared argument it prefixes is %s — did you mean %s?",
			from, tool.Path(), to, to),
		Changed: cat.change(tool.Path(), to),
		Source:  in.Source,
		Fix:     addKey(Call{}, tool, to),
	}, true
}

func sortedSent(sent map[string]bool) []string {
	out := make([]string, 0, len(sent))
	for k := range sent {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}
