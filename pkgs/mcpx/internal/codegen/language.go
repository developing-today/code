package codegen

// Language is one codegen target (#1917, Approach C): everything a surface
// needs to name about a script language in one table entry — the generated
// client's file name, the ambient-declarations file name, the script
// extension, and the emitters that render the client module, the ambient
// declarations, and the signature block from the same namespace inputs.
//
// Until this type existed, the TypeScript target was implied by whichever
// function a caller happened to reach for, and every surface that named
// ".ts" or "mcpx-client.ts" named it on its own — so adding a language was
// an audit, not an entry. Now the existing package-level emitters are the
// TypeScript entry's emitters, reached through the table; a new language
// (Python is the queued follow-on, #1917 Phase 1 remainder) is one more
// entry with its own emitters, and callers that ask the Language for file
// names never spell a foreign target's.
//
// The emitters are fields rather than an interface because a target is
// data plus three renderings, and a table literal states both at the
// place a reader looks for them.
type Language struct {
	// Name is the registry key: "typescript", and in time "python".
	Name string
	// ClientFile is the generated client module's name on disk. For
	// TypeScript it must stay runner.ClientFileName — the runner writes
	// the module under this name and generated launchers import it by
	// this name.
	ClientFile string
	// GlobalsFile is the ambient declarations file's name on disk. For
	// TypeScript it must stay runner.GlobalsFileName.
	GlobalsFile string
	// ScriptExt is the extension a script in this language carries.
	ScriptExt string

	module       func(nss []Namespace, endpoint, session string) string
	globals      func(nss []Namespace) string
	declarations func(nss []Namespace) string
}

// typescriptClientFile is a constant because GlobalDeclarations spells it
// in its output: naming the var there would make the var's initialisation
// (which references the function) a cycle.
const typescriptClientFile = "mcpx-client.ts"

// TypeScript is the original target, expressed through the Language
// dimension with no behaviour change: its emitters are the package-level
// Module, GlobalDeclarations and Declarations functions, so every existing
// caller's output is byte-identical.
var TypeScript = Language{
	Name:         "typescript",
	ClientFile:   typescriptClientFile,
	GlobalsFile:  "mcpx-globals.d.ts",
	ScriptExt:    ".ts",
	module:       Module,
	globals:      GlobalDeclarations,
	declarations: Declarations,
}

// Languages is the registry of codegen targets, in preference order.
var Languages = []Language{TypeScript, Python}

// Default is the target every surface that does not name one means: the
// only target there has ever been.
func Default() Language { return TypeScript }

// Lookup finds a target by name.
func Lookup(name string) (Language, bool) {
	for _, l := range Languages {
		if l.Name == name {
			return l, true
		}
	}
	return Language{}, false
}

// EmitModule renders the importable client module for this target.
func (l Language) EmitModule(nss []Namespace, endpoint, session string) string {
	return l.module(nss, endpoint, session)
}

// EmitGlobals renders the ambient declarations for this target.
func (l Language) EmitGlobals(nss []Namespace) string {
	return l.globals(nss)
}

// EmitDeclarations renders the signature block for this target (what
// `mcpx types` prints for the language).
func (l Language) EmitDeclarations(nss []Namespace) string {
	return l.declarations(nss)
}
