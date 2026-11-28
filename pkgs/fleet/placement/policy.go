// Package placement implements the placement-record PoC for issue #4986:
// it consumes the machine-readable placement policy (placement-policy.yaml,
// the #2024 framework's decision table as data) and, for one dispatch
// described by answers to the framework's decision questions, emits the
// placement record the policy declares (option, rule, pool_read_at,
// supervision).
//
// PoC scope: ONE dispatch path — the queue-worker handoff. This package is
// deliberately not wired into the live queue client or the OpenChamber
// dispatcher; per the #2024 program rule the PoC is reviewed before any
// permanent rollout.
package placement

import (
	"fmt"
	"os"

	"gopkg.in/yaml.v3"
)

// Version is a dotted policy version. The v1.0 policy file stated it as
// a YAML int (e.g. 1); the v1.1 amendment (#2179) states it as a string
// (e.g. "1.1"). Both decode; String renders the canonical dotted form.
type Version struct {
	Major int
	Minor int
}

// UnmarshalYAML accepts a YAML int or a "MAJOR[.MINOR]" string.
func (v *Version) UnmarshalYAML(node *yaml.Node) error {
	var n int
	if err := node.Decode(&n); err == nil {
		v.Major, v.Minor = n, 0
		return nil
	}
	var s string
	if err := node.Decode(&s); err != nil {
		return fmt.Errorf("policy version must be an int or dotted string: %w", err)
	}
	if _, err := fmt.Sscanf(s, "%d.%d", &v.Major, &v.Minor); err != nil {
		var major int
		if _, err2 := fmt.Sscanf(s, "%d", &major); err2 != nil {
			return fmt.Errorf("policy version %q is not an int or dotted version", s)
		}
		v.Major, v.Minor = major, 0
	}
	return nil
}

// String renders the canonical dotted form ("1" or "1.1").
func (v Version) String() string {
	if v.Minor == 0 {
		return fmt.Sprintf("%d", v.Major)
	}
	return fmt.Sprintf("%d.%d", v.Major, v.Minor)
}

// Option is one placement option from the policy's options list.
type Option struct {
	ID          string `yaml:"id"`
	DocID       string `yaml:"doc_id"`
	Name        string `yaml:"name"`
	CostPool    string `yaml:"cost_pool"`
	Supervision string `yaml:"supervision"`
}

// Rule is one decision-table rule.
type Rule struct {
	ID              string `yaml:"id"`
	EvaluationOrder int    `yaml:"evaluation_order"`
	ViaQuestion     string `yaml:"via_question"`
	Option          string `yaml:"option"`
	Default         bool   `yaml:"default"`
}

// Guard is the R2 guard: options excluded before placement rules run.
type Guard struct {
	ID     string `yaml:"id"`
	Effect struct {
		ExcludedOptions []string `yaml:"excluded_options"`
		ContinueAt      string   `yaml:"continue_at"`
	} `yaml:"effect"`
}

// Pool is one cost pool; DrawnBy lists the options that draw on it.
type Pool struct {
	ID      string   `yaml:"id"`
	Name    string   `yaml:"name"`
	DrawnBy []string `yaml:"drawn_by"`
}

// Policy is the subset of placement-policy.yaml the evaluator consumes.
type Policy struct {
	Version           Version  `yaml:"version"`
	SourceIssue       int      `yaml:"source_issue"`
	ImplementingIssue int      `yaml:"implementing_issue"`
	Options           []Option `yaml:"options"`
	Guard             Guard    `yaml:"guard"`
	Rules             []Rule   `yaml:"rules"`
	Pools             []Pool   `yaml:"pools"`
	BurnRules         []struct {
		ID         string `yaml:"id"`
		Mechanical struct {
			QuotaReadMaxAgeMinutes *int `yaml:"quota_read_max_age_minutes"`
		} `yaml:"mechanical"`
	} `yaml:"burn_rules"`

	optionByID map[string]Option
	ruleByID   map[string]Rule
}

// Load reads and parses a placement-policy.yaml file.
func Load(path string) (*Policy, error) {
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read policy: %w", err)
	}
	var p Policy
	if err := yaml.Unmarshal(raw, &p); err != nil {
		return nil, fmt.Errorf("parse policy: %w", err)
	}
	if err := p.index(); err != nil {
		return nil, err
	}
	return &p, nil
}

func (p *Policy) index() error {
	if p.Version.Major == 0 {
		return fmt.Errorf("policy: missing version")
	}
	p.optionByID = make(map[string]Option, len(p.Options))
	for _, o := range p.Options {
		if o.ID == "" || o.Supervision == "" {
			return fmt.Errorf("policy: option with empty id or supervision")
		}
		p.optionByID[o.ID] = o
	}
	p.ruleByID = make(map[string]Rule, len(p.Rules))
	for _, r := range p.Rules {
		if _, ok := p.optionByID[r.Option]; !ok {
			return fmt.Errorf("policy: rule %s names unknown option %q", r.ID, r.Option)
		}
		p.ruleByID[r.ID] = r
	}
	for _, id := range p.Guard.Effect.ExcludedOptions {
		if _, ok := p.optionByID[id]; !ok {
			return fmt.Errorf("policy: guard excludes unknown option %q", id)
		}
	}
	return nil
}

// Option returns the named option.
func (p *Policy) Option(id string) (Option, bool) {
	o, ok := p.optionByID[id]
	return o, ok
}

// Rule returns the named rule.
func (p *Policy) Rule(id string) (Rule, bool) {
	r, ok := p.ruleByID[id]
	return r, ok
}

// nonQuotaPools are the pools whose "draw" is not a quota read the B5
// measurement gate can measure: Actions minutes and the Jules account.
// The policy's own placement_record schema states the consequence:
// pool_read_at is "n/a" for non-pool options (Actions minutes, Jules) —
// even though the pools section lists those options under these pools'
// drawn_by for cost accounting.
var nonQuotaPools = map[string]bool{
	"github-actions-minutes": true,
	"jules-account":          true,
}

// PoolDrawn reports whether the option draws on a quota-measured cost
// pool (and so needs a pool_read_at timestamp rather than "n/a").
func (p *Policy) PoolDrawn(optionID string) bool {
	for _, pool := range p.Pools {
		if nonQuotaPools[pool.ID] {
			continue
		}
		for _, drawn := range pool.DrawnBy {
			if drawn == optionID {
				return true
			}
		}
	}
	return false
}

// QuotaReadMaxAgeMinutes returns the B5 measurement gate from the policy,
// defaulting to 60 when the policy does not state it.
func (p *Policy) QuotaReadMaxAgeMinutes() int {
	for _, b := range p.BurnRules {
		if b.ID == "B5" && b.Mechanical.QuotaReadMaxAgeMinutes != nil {
			return *b.Mechanical.QuotaReadMaxAgeMinutes
		}
	}
	return 60
}
