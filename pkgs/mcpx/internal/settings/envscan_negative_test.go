package settings_test

import (
	"os"
	"path/filepath"
	"testing"
)

// The environment guards are only as good as the scanners behind them.
//
// A scanner that stops recognising a shape passes silently -- the exact
// failure this family of guards exists to prevent (#56: a regex that could
// not see the spelling gofmt writes) -- so each scanner is fed a planted
// violation it must report and an innocent neighbour it must not. These are
// the negative tests for the checker-style guards in consumed_test.go
// (issue #1971, remainder item 3).

func writeFixture(t *testing.T, root, rel, body string) {
	t.Helper()
	p := filepath.Join(root, filepath.FromSlash(rel))
	if err := os.MkdirAll(filepath.Dir(p), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(p, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
}

func scanNames(sites []envSite) map[string]envSite {
	out := map[string]envSite{}
	for _, s := range sites {
		out[s.name] = s
	}
	return out
}

func TestTheEnvScanFindsAPlantedViolation(t *testing.T) {
	root := t.TempDir()
	writeFixture(t, root, "internal/plant/plant.go", `package plant

import (
	"os"
	"strings"
)

const plantedRead = "MCPX_PLANTED_READ"

func ByConstant() string { return os.Getenv(plantedRead) }
func Direct() string     { return os.Getenv("MCPX_PLANTED_DIRECT") }
func Written()           { os.Setenv("MCPX_PLANTED_SET", "1") }

func Dynamic(name string) string {
	return os.Getenv("MCPX_" + strings.ToUpper(name))
}
`)

	scan := scanGoEnv(t, root)
	reads := scanNames(scan.reads)
	for _, name := range []string{"MCPX_PLANTED_READ", "MCPX_PLANTED_DIRECT"} {
		if _, ok := reads[name]; !ok {
			t.Errorf("the scan did not report a planted read of %s; reads were %v", name, scan.reads)
		}
	}
	if _, ok := scanNames(scan.sets)["MCPX_PLANTED_SET"]; !ok {
		t.Errorf("the scan did not report a planted write of MCPX_PLANTED_SET; sets were %v", scan.sets)
	}
	if len(scan.dynamic) == 0 {
		t.Error("the scan did not report a variable name built at run time from MCPX_")
	}
}

func TestTheEnvScanIgnoresInnocentShapes(t *testing.T) {
	root := t.TempDir()
	writeFixture(t, root, "internal/plant/clean.go", `package plant

import "os"

func Clean() string { return os.Getenv("HOME") }

func Commented() string {
	// os.Getenv("MCPX_COMMENTED_OUT") is prose, not a read.
	return ""
}
`)
	// Tests are excluded on purpose: a read only a test performs proves
	// nothing about the program.
	writeFixture(t, root, "internal/plant/clean_test.go", `package plant

import "os"

func readsInATest() string { return os.Getenv("MCPX_READ_BY_TEST_ONLY") }
`)
	writeFixture(t, root, "vendor/dep/dep.go", `package dep

import "os"

func Vendored() string { return os.Getenv("MCPX_VENDORED") }
`)
	writeFixture(t, root, "testsupport/harness/main.go", `package main

import "os"

func Harness() string { return os.Getenv("MCPX_HARNESS_ONLY") }
`)

	scan := scanGoEnv(t, root)
	if n := len(scan.reads) + len(scan.sets) + len(scan.dynamic) + len(scan.template); n != 0 {
		t.Errorf("the scan reported %d site(s) in files it must ignore: "+
			"reads %v, sets %v, dynamic %v, template %v",
			n, scan.reads, scan.sets, scan.dynamic, scan.template)
	}
}

func TestThePluginScanFindsAPlantedViolation(t *testing.T) {
	root := t.TempDir()
	writeFixture(t, root, "plugin/plant.ts", `
const property = process.env.MCPX_PLANTED_PLUGIN;
const indexed = process.env["MCPX_PLANTED_INDEX"];
const innocent = process.env.HOME;
env.set("MCPX_PLANTED_SET", "1");
// process.env.MCPX_PLANTED_COMMENT is prose, not a read.
`)
	writeFixture(t, root, "plugin/plant.test.ts", `
const inATest = process.env.MCPX_PLANTED_TEST_ONLY;
`)

	sets, reads := scanPluginEnv(t, root)
	got := scanNames(reads)
	for _, name := range []string{"MCPX_PLANTED_PLUGIN", "MCPX_PLANTED_INDEX"} {
		if _, ok := got[name]; !ok {
			t.Errorf("the plugin scan did not report %s; reads were %v", name, reads)
		}
	}
	if _, ok := scanNames(sets)["MCPX_PLANTED_SET"]; !ok {
		t.Errorf("the plugin scan did not report a planted set of MCPX_PLANTED_SET; sets were %v", sets)
	}
	for _, name := range []string{"MCPX_PLANTED_TEST_ONLY", "MCPX_PLANTED_COMMENT"} {
		if site, ok := got[name]; ok {
			t.Errorf("the plugin scan reported %s at %s, which is not a source read", name, site.at())
		}
	}
}
