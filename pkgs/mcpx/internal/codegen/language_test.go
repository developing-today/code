package codegen

import "testing"

// The Language dimension (#1917 Approach C): the TypeScript target is a
// table entry whose emitters are the package-level functions, so routing
// through the entry must change no output byte.

func fixtureNamespaces() []Namespace {
	return []Namespace{{
		Name:   "demo",
		Server: "demo-server",
		Tools: []Tool{{
			Name:        "ping",
			Description: "Ping the server.",
			InputSchema: []byte(`{"type":"object","properties":{"msg":{"type":"string"}}}`),
		}},
	}}
}

func TestTypeScriptIsTheDefaultLanguage(t *testing.T) {
	if Default().Name != "typescript" {
		t.Fatalf("default language = %q, want typescript", Default().Name)
	}
	l, ok := Lookup("typescript")
	if !ok || l.Name != "typescript" {
		t.Fatalf("Lookup(typescript) = %+v, %v", l, ok)
	}
	if _, ok := Lookup("python"); ok {
		t.Fatal("python is the #1917 follow-on slice; it must not be registered before its emitters exist")
	}
}

func TestTypeScriptFileNamesMatchTheRunnerContract(t *testing.T) {
	// runner.ClientFileName / runner.GlobalsFileName are the on-disk names
	// the runner writes and launchers import; the Language entry must
	// spell the same names (asserted as literals here because codegen does
	// not import runner).
	if TypeScript.ClientFile != "mcpx-client.ts" {
		t.Errorf("ClientFile = %q, want mcpx-client.ts", TypeScript.ClientFile)
	}
	if TypeScript.GlobalsFile != "mcpx-globals.d.ts" {
		t.Errorf("GlobalsFile = %q, want mcpx-globals.d.ts", TypeScript.GlobalsFile)
	}
	if TypeScript.ScriptExt != ".ts" {
		t.Errorf("ScriptExt = %q, want .ts", TypeScript.ScriptExt)
	}
}

func TestLanguageEmittersMatchPackageFunctions(t *testing.T) {
	nss := fixtureNamespaces()
	if got, want := TypeScript.EmitModule(nss, "http://127.0.0.1:1", "sess"), Module(nss, "http://127.0.0.1:1", "sess"); got != want {
		t.Error("EmitModule differs from Module for the TypeScript entry")
	}
	if got, want := TypeScript.EmitGlobals(nss), GlobalDeclarations(nss); got != want {
		t.Error("EmitGlobals differs from GlobalDeclarations for the TypeScript entry")
	}
	if got, want := TypeScript.EmitDeclarations(nss), Declarations(nss); got != want {
		t.Error("EmitDeclarations differs from Declarations for the TypeScript entry")
	}
}
