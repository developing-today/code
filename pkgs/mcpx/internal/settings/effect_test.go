package settings_test

import (
	"strings"
	"testing"

	"github.com/dezren39/mcpx/internal/settings"
)

// The completeness half of the effect guard (#1971, Approach C): every
// setting in the registry carries either an effect probe or a reasoned
// exemption. This runs at unit speed - it enumerates settings.Registry()
// and consults the maps in effect.go; it starts no daemon and runs no
// probe. The probes themselves live in internal/e2e.

func TestEverySettingHasAnEffectProbeOrAnExemption(t *testing.T) {
	violations := settings.CheckEffectCoverage(
		settings.Registry(), settings.EffectProbes, settings.EffectExemptions)
	if len(violations) > 0 {
		t.Fatalf("effect coverage violations:\n  %s", strings.Join(violations, "\n  "))
	}
}

// The ratchet: the exemption list can only shrink. Growing it means a new
// setting arrived without a probe - exactly what the guard exists to
// refuse. Shrinking it is the goal, but the shrink must be recorded by
// lowering the pin in the same PR, never absorbed silently.
func TestEffectExemptionsOnlyShrink(t *testing.T) {
	got := len(settings.EffectExemptions)
	switch {
	case got > settings.EffectExemptionPin:
		t.Fatalf("effect exemptions grew to %d, above the pinned %d: "+
			"a new exemption needs a probe instead, or a deliberate pin "+
			"raise with the reason recorded in the PR", got, settings.EffectExemptionPin)
	case got < settings.EffectExemptionPin:
		t.Fatalf("effect exemptions shrank to %d, below the pinned %d: "+
			"lower EffectExemptionPin to %d in this PR so the shrink is recorded",
			got, settings.EffectExemptionPin, got)
	}
}

// plantedListingLimit is the registry.pageSize shape (#179) in miniature:
// the setting's value is read, passed along, and then consulted only
// behind a branch the ordinary caller never takes, because it always
// passes a positive limit. Every read-based guard passes this code.
func plantedListingLimit(read func() int, limit int) int {
	pageSize := read()
	if limit <= 0 {
		return pageSize
	}
	return limit
}

// The negative test. A guard that cannot fail is not a guard, so this
// plants an inert setting - read, in the pageSize shape above - into a
// fixture registry and requires the completeness machinery to fail on
// it. It also pins the machinery's other edges: a probe/exemption pair
// is a violation, an exemption without a reason is a violation, and a
// probed fixture passes as the control.
func TestEffectGuardFailsOnAPlantedInertSetting(t *testing.T) {
	planted := settings.Setting{Path: "planted.pageSize", Kind: settings.KindInt, Default: "30"}

	// First prove the plant really is inert: whatever the setting says,
	// an ordinary caller passing a positive limit gets its limit back.
	for _, limit := range []int{1, 2, 30} {
		if got := plantedListingLimit(func() int { return 999 }, limit); got != limit {
			t.Fatalf("plant should be inert: limit %d produced %d", limit, got)
		}
		if got := plantedListingLimit(func() int { return 7 }, limit); got != limit {
			t.Fatalf("plant should be inert: limit %d produced %d", limit, got)
		}
	}

	// The read-based guard would pass the plant (it IS read, above). The
	// effect guard must not: the plant has neither a probe nor an
	// exemption, so coverage of a registry containing it must fail,
	// naming it.
	reg := append(append([]settings.Setting{}, settings.Registry()...), planted)
	violations := settings.CheckEffectCoverage(reg, settings.EffectProbes, settings.EffectExemptions)
	found := false
	for _, v := range violations {
		if strings.Contains(v, "planted.pageSize") {
			found = true
		}
	}
	if !found {
		t.Fatalf("the guard passed a planted inert setting; violations were %v", violations)
	}

	mini := []settings.Setting{{Path: "mini.setting"}}

	// Control: a probed fixture passes.
	if v := settings.CheckEffectCoverage(mini,
		map[string]string{"mini.setting": "TestSomeProbe"}, nil); len(v) != 0 {
		t.Fatalf("a probed fixture should pass, got %v", v)
	}
	// Both a probe and an exemption is a violation.
	if v := settings.CheckEffectCoverage(mini,
		map[string]string{"mini.setting": "TestSomeProbe"},
		map[string]string{"mini.setting": "a reason"}); len(v) == 0 {
		t.Fatal("a setting with both a probe and an exemption should fail")
	}
	// An exemption without a reason is a violation.
	if v := settings.CheckEffectCoverage(mini, nil,
		map[string]string{"mini.setting": "  "}); len(v) == 0 {
		t.Fatal("an exemption with no reason should fail")
	}
	// A probe for a setting that does not exist is a violation.
	if v := settings.CheckEffectCoverage(mini,
		map[string]string{"mini.setting": "TestSomeProbe", "mini.ghost": "TestGhost"}, nil); len(v) == 0 {
		t.Fatal("a probe naming no live setting should fail")
	}
}
