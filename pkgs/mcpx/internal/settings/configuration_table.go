package settings

import (
	"fmt"
	"sort"
	"strings"
)

// ConfigurationTable is the generated inventory inside
// docs/configuration.md, moved out of the settings tests so the one
// consolidated golden test in internal/refdoc guards every committed
// generated region from a single place (#4909). The rendering is
// unchanged.

// ConfigurationTable renders every setting, grouped by the first segment of its
// path, which is how someone looks one up: they know the area, not the leaf.
func ConfigurationTable(all []Setting) string {
	groups := map[string][]Setting{}
	for _, set := range all {
		groups[strings.SplitN(set.Path, ".", 2)[0]] = append(groups[strings.SplitN(set.Path, ".", 2)[0]], set)
	}
	names := make([]string, 0, len(groups))
	for n := range groups {
		names = append(names, n)
	}
	sort.Strings(names)

	var b strings.Builder
	for _, n := range names {
		fmt.Fprintf(&b, "### %s\n\n", n)
		b.WriteString("| setting | kind | default | scope | hot | flag | variable | governs |\n" +
			"| --- | --- | --- | --- | --- | --- | --- | --- |\n")
		for _, set := range groups[n] {
			def := "`" + set.Default + "`"
			if set.Default == "" {
				// An empty string is a value; an empty cell reads as an
				// omission. The two were spelled differently in the
				// hand-written table, which is how it was clear nothing
				// generated it.
				def = "*(empty)*"
			}
			flags := []string{"`--" + set.FlagName() + "`"}
			for _, a := range set.FlagAliases {
				flags = append(flags, "`--"+a+"`")
			}
			envs := []string{"`" + set.EnvName() + "`"}
			for _, a := range set.EnvAliases {
				envs = append(envs, "`"+a+"`")
			}
			governs := set.Short
			if len(set.PathAliases) > 0 {
				governs += "; path alias: " + strings.Join(set.PathAliases, ", ")
			}
			if len(set.EnumAliases) > 0 {
				// Only where aliases exist: the canonical names are what a
				// reader should write, and the aliases are what they may
				// find in an older file.
				governs += "; one of " + set.EnumWords()
			}
			if set.Plumbing {
				governs = "*(plumbing)* " + governs
			}
			fmt.Fprintf(&b, "| `%s` | %s | %s | %s | %s | %s | %s | %s |\n",
				set.Path, set.Kind, def, set.Scope, yesNo(set.Hot),
				strings.Join(flags, ", "), strings.Join(envs, ", "),
				strings.ReplaceAll(governs, "|", "\\|"))
		}
		b.WriteString("\n")
	}
	return b.String()
}

func yesNo(b bool) string {
	if b {
		return "yes"
	}
	return "no"
}
