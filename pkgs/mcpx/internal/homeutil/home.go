// Package homeutil is mcpx's one home-resolution helper.
//
// Home-resolution contract (issues #5762 / #5763): a tool that needs
// the home directory resolves it exactly one way - a non-empty HOME.
// Empty is unset, and unset is a named error at the first surface
// that needs a path, never a substitute directory (no root
// anchoring, no CWD fallback, no literal "~", no hardcoded user,
// no passwd entry). Before this package, mcpx discarded the
// os.UserHomeDir error at several sites (home, _ := ...), which on
// Unix yields "" and silently root- or CWD-anchored joins.
package homeutil

import (
	"fmt"
	"os"
)

// Home returns the user's home directory, or a named error when
// HOME is unset or empty.
func Home() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", fmt.Errorf("HOME is not set or is empty: mcpx requires a non-empty HOME to resolve its paths (home-resolution contract, issues #5762/#5763): %w", err)
	}
	if home == "" {
		return "", fmt.Errorf("HOME is not set or is empty: mcpx requires a non-empty HOME to resolve its paths (home-resolution contract, issues #5762/#5763)")
	}
	return home, nil
}

// MustHome is Home for the surfaces that cannot return an error
// (package-level path resolvers whose callers have no error
// channel). It panics with the named error rather than letting a
// missing HOME silently become a substitute directory - the same
// loud failure the contract's Rust embodiment (atlas-search,
// issue #5640) uses.
func MustHome() string {
	home, err := Home()
	if err != nil {
		panic(err)
	}
	return home
}
