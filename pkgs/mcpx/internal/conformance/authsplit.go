package conformance

import (
	"encoding/json"
	"fmt"
	"os"
	"sort"
	"strings"
)

// AuthSplitPath is the split of the authorization rows along accepts-vs-
// refuses, one verdict per requirement side. Like overrides/, it is data:
// the matrix renders it, and it never decides a cell's status.
const AuthSplitPath = "internal/conformance/authsplit.json"

// The two verdicts for an authorization row-side.
const (
	// Permissive is satisfied by an interface that accepts every credential
	// (#1950): endpoint shape, discovery documents, registration, flows.
	Permissive = "permissive"
	// Validation is not satisfiable by a permissive interface: it requires
	// inspecting a presented credential and refusing it (#1972).
	Validation = "validation"
)

// AuthChecks are the validation checks a refusal-requiring row can need,
// matching #1972's split table.
var AuthChecks = []string{"invalid", "expired", "wrong-resource", "scope"}

// AuthSide is the split verdict for one side of one requirement.
type AuthSide struct {
	Verdict string   `json:"verdict"`
	Checks  []string `json:"checks,omitempty"`
	Why     string   `json:"why,omitempty"`
}

// AuthCell keys the split by requirement and side: a row is split per side
// of the wire, since a permissive server and a permissive client answer
// different rows.
type AuthCell struct{ ID, Side string }

type authSplitFile struct {
	About  string                         `json:"about"`
	Checks []string                       `json:"checks"`
	Rows   map[string]map[string]AuthSide `json:"rows"`
}

// IsAuthorization reports whether a requirement is an authorization row.
func IsAuthorization(r Requirement) bool {
	return strings.Contains(strings.ToLower(r.Area), "authorization")
}

// LoadAuthSplit reads the split and refuses one that does not cover exactly
// the bound sides of the authorization rows.
func LoadAuthSplit(path string, reqs []Requirement) (map[AuthCell]AuthSide, error) {
	b, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var f authSplitFile
	if err := json.Unmarshal(b, &f); err != nil {
		return nil, fmt.Errorf("%s: %w", path, err)
	}
	if strings.Join(f.Checks, ",") != strings.Join(AuthChecks, ",") {
		return nil, fmt.Errorf("%s: checks are %v, want %v", path, f.Checks, AuthChecks)
	}

	byID := map[string]Requirement{}
	for _, r := range reqs {
		byID[r.ID] = r
	}
	out := map[AuthCell]AuthSide{}
	for id, sides := range f.Rows {
		r, ok := byID[id]
		if !ok {
			return nil, fmt.Errorf("%s: unknown requirement %s", path, id)
		}
		if !IsAuthorization(r) {
			return nil, fmt.Errorf("%s: %s is not an authorization row (%s)", path, id, r.Area)
		}
		for side, s := range sides {
			k := AuthCell{id, side}
			if side != Server && side != Client {
				return nil, fmt.Errorf("%s: %s: side %q", path, id, side)
			}
			if strings.HasPrefix(r.Applicability[side], NAPrefix) {
				return nil, fmt.Errorf("%s: %s/%s does not bind mcpx", path, id, side)
			}
			switch s.Verdict {
			case Permissive:
				if len(s.Checks) > 0 {
					return nil, fmt.Errorf("%s: %s/%s is permissive but names checks %v", path, id, side, s.Checks)
				}
			case Validation:
				if len(s.Checks) == 0 {
					return nil, fmt.Errorf("%s: %s/%s requires validation but names no check", path, id, side)
				}
				for _, c := range s.Checks {
					if !isAuthCheck(c) {
						return nil, fmt.Errorf("%s: %s/%s: unknown check %q", path, id, side, c)
					}
				}
			default:
				return nil, fmt.Errorf("%s: %s/%s: verdict %q", path, id, side, s.Verdict)
			}
			out[k] = s
		}
	}

	var missing []string
	for _, r := range reqs {
		if !IsAuthorization(r) {
			continue
		}
		for _, side := range []string{Server, Client} {
			if strings.HasPrefix(r.Applicability[side], NAPrefix) {
				continue
			}
			if _, ok := out[AuthCell{r.ID, side}]; !ok {
				missing = append(missing, r.ID+"/"+side)
			}
		}
	}
	if len(missing) > 0 {
		sort.Strings(missing)
		return nil, fmt.Errorf("%s: %d bound authorization sides have no verdict:\n%s",
			path, len(missing), strings.Join(missing, "\n"))
	}
	return out, nil
}

func isAuthCheck(c string) bool {
	for _, x := range AuthChecks {
		if x == c {
			return true
		}
	}
	return false
}

// AuthCounts is the size of the split.
type AuthCounts struct {
	Rows       int            // authorization rows bound on at least one side
	Sides      int            // bound row-sides
	Permissive int            // row-sides a permissive interface satisfies
	Validation int            // row-sides that need a real check
	PerCheck   map[string]int // validation row-sides per check
}

// CountAuthSplit counts the split.
func CountAuthSplit(reqs []Requirement, sp map[AuthCell]AuthSide) AuthCounts {
	c := AuthCounts{PerCheck: map[string]int{}}
	bound := map[string]bool{}
	for id, s := range sp {
		c.Sides++
		bound[id.ID] = true
		if s.Verdict == Permissive {
			c.Permissive++
		} else {
			c.Validation++
			for _, x := range s.Checks {
				c.PerCheck[x]++
			}
		}
	}
	c.Rows = len(bound)
	return c
}

// renderAuthSplit writes the split into the matrix, so the permissive
// interface's target and the validation work's target are both exact.
func renderAuthSplit(b *strings.Builder, reqs []Requirement, sp map[AuthCell]AuthSide) {
	c := CountAuthSplit(reqs, sp)
	fmt.Fprintf(b, "\n## Authorization rows: accepts, or requires refusal (#1972)\n\n")
	fmt.Fprintf(b, "%d authorization rows are split per side of the wire into %d bound row-sides: **%d permissive** and **%d validation**. ",
		c.Rows, c.Sides, c.Permissive, c.Validation)
	b.WriteString("A *permissive* row-side is satisfied by an interface that accepts every credential (#1950) — endpoint shape, discovery documents, registration, flows. ")
	b.WriteString("A *validation* row-side cannot pass against such an interface: it needs a real check, named below, to refuse or challenge a credential (#1972).\n\n")
	fmt.Fprintf(b, "Validation row-sides per check: invalid %d, expired %d, wrong-resource %d, scope %d. ",
		c.PerCheck["invalid"], c.PerCheck["expired"], c.PerCheck["wrong-resource"], c.PerCheck["scope"])
	fmt.Fprintf(b, "Input: `%s`; edits belong there, never in this file.\n\n", AuthSplitPath)

	b.WriteString("| requirement | side | verdict |\n|---|---|---|\n")
	for _, r := range reqs {
		if !IsAuthorization(r) {
			continue
		}
		for _, side := range []string{Server, Client} {
			s, ok := sp[AuthCell{r.ID, side}]
			if !ok {
				continue
			}
			v := "permissive"
			if s.Verdict == Validation {
				v = "validation: " + strings.Join(s.Checks, ", ")
			}
			if s.Why != "" {
				v += " — " + mdEscape(s.Why)
			}
			fmt.Fprintf(b, "| %s | %s | %s |\n", r.ID, side, v)
		}
	}
}
