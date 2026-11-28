package cli

import "sort"

// HandFlag is one flag a command declares by hand on its own flag set,
// rather than through the settings registry: --runtime on run, --since on
// log, and about 190 more like them.
//
// Those declarations were the last hand-kept list in the CLI (#2516, item 8
// of #1883). Commands() single-sources dispatch, the settings registry
// single-sources settings flags, and both feed help, the man page and
// completion -- but a hand-declared flag surfaced only in the flag package's
// own -h output, which parseFlags deliberately replaces with the command's
// help. So the flags a command most distinctively owns were the ones its
// help did not mention.
type HandFlag struct {
	Name  string
	Usage string
}

// HandFlagGroup is the hand-declared flags of one flag set. Scope is the
// flag set's name: usually a command ("run"), sometimes a subcommand
// spelling ("log record") whose help is its parent command's.
type HandFlagGroup struct {
	Scope string
	Flags []HandFlag
}

// handFlagGroups returns the hand-declared flags that belong to a command:
// the group named for the command itself, then one group per subcommand
// flag set under it, each in declaration order as generated. The index
// itself is flagindex_generated.go, extracted from the declarations by
// TestHandFlagIndexIsCurrent; nothing outside that test writes it.
func handFlagGroups(command string) []HandFlagGroup {
	var scopes []string
	for scope := range handFlagIndex {
		if scope == command || len(scope) > len(command) && scope[:len(command)+1] == command+" " {
			scopes = append(scopes, scope)
		}
	}
	sort.Strings(scopes)
	groups := make([]HandFlagGroup, 0, len(scopes))
	for _, scope := range scopes {
		groups = append(groups, HandFlagGroup{Scope: scope, Flags: handFlagIndex[scope]})
	}
	return groups
}

// handFlagNames is every hand-declared flag name a command accepts, across
// its own and its subcommands' flag sets. Completion completes against the
// first word only, so the subcommand flags belong in the parent's list.
func handFlagNames(command string) []string {
	seen := map[string]bool{}
	var names []string
	for _, g := range handFlagGroups(command) {
		for _, f := range g.Flags {
			if !seen[f.Name] {
				seen[f.Name] = true
				names = append(names, f.Name)
			}
		}
	}
	sort.Strings(names)
	return names
}
