package settings_test

import (
	"flag"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/settings"
)

func pathAliasSchema(t *testing.T) *settings.Schema {
	t.Helper()
	s, err := settings.New([]settings.Setting{{
		Path: "new.name", PathAliases: []string{"old.name"},
		Kind: settings.KindEnum, Enum: []string{"off", "types"},
		EnumAliases: map[string][]string{"types": {"on"}}, Default: "off",
		FlagAliases: []string{"old-name"}, EnvAliases: []string{"MCPX_OLD_NAME"},
		Name: "Name", Short: "name",
	}})
	if err != nil {
		t.Fatalf("build alias schema: %v", err)
	}
	return s
}

func TestPathAliasNormalizesAndStoresCanonicalPathFromEverySource(t *testing.T) {
	s := pathAliasSchema(t)
	sources := map[string]func(*settings.Set) error{
		"file": func(set *settings.Set) error {
			return s.ApplyFile(set, map[string]any{"old": map[string]any{"name": "on"}}, "/x/.mcpx.json", 0)
		},
		"env": func(set *settings.Set) error {
			return s.ApplyEnv(set, []string{"MCPX_OLD_NAME=on"})
		},
		"flag": func(set *settings.Set) error {
			fs := flag.NewFlagSet("run", flag.ContinueOnError)
			fs.SetOutput(nopWriter{})
			b := s.Bind(fs, "run")
			if err := fs.Parse([]string{"--old-name", "on"}); err != nil {
				return err
			}
			return b.ApplyTo(set)
		},
		"runtime": func(set *settings.Set) error { return set.SetRuntime("old.name", "on") },
	}
	for name, apply := range sources {
		t.Run(name, func(t *testing.T) {
			set := settings.NewSet(s)
			if err := apply(set); err != nil {
				t.Fatalf("apply alias: %v", err)
			}
			if got := set.String("new.name"); got != "types" {
				t.Errorf("canonical path = %q, want types", got)
			}
			if got := set.String("old.name"); got != "types" {
				t.Errorf("alias lookup = %q, want types", got)
			}
			if value, ok := set.Value("old.name"); !ok || value.Path != "new.name" {
				t.Errorf("value = %#v, %v; want canonical path", value, ok)
			}
		})
	}
}

func TestPathAliasCanonicalizesCallOverrides(t *testing.T) {
	s := pathAliasSchema(t)
	base := settings.NewSet(s)
	view := base.WithOverrides(map[string]string{"old.name": "types"}, "request")
	if got := view.String("new.name"); got != "types" {
		t.Errorf("canonical path = %q, want types", got)
	}
	if got := view.String("old.name"); got != "types" {
		t.Errorf("alias lookup = %q, want types", got)
	}
}

func TestPathAliasesCannotCollideWithCanonicalOrOtherAliasPaths(t *testing.T) {
	base := settings.Setting{Path: "new.name", PathAliases: []string{"old.name"}, Kind: settings.KindString, Name: "Name", Short: "name"}
	for name, list := range map[string][]settings.Setting{
		"canonical path": {base, {Path: "old.name", Kind: settings.KindString, Name: "Old", Short: "old"}},
		"another alias":  {base, {Path: "other.name", PathAliases: []string{"old.name"}, Kind: settings.KindString, Name: "Other", Short: "other"}},
		"empty alias":    {{Path: "new.name", PathAliases: []string{""}, Kind: settings.KindString, Name: "Name", Short: "name"}},
	} {
		t.Run(name, func(t *testing.T) {
			if _, err := settings.New(list); err == nil || !strings.Contains(err.Error(), "path") {
				t.Errorf("invalid alias declaration accepted: %v", err)
			}
		})
	}
}
