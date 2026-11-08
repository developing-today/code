// Script language selection for a run (#1917 Phase 2).
//
// The dispatch order in dispatch.go answers "what executes this file"; this
// file answers "what is this file written in", which is the question the
// runner asks before installing a generated client. The two must agree: the
// client a script gets is the client of the language the dispatch chose, so
// a Python script is run by a Python interpreter with the Python client
// beside it, never by a JavaScript runtime with no client at all.
//
// The evidence is the same evidence dispatch uses -- a shebang first, then
// the file extension. An explicit selection (the script.language setting,
// the --language flag, or ExecOptions.language) beats that detection for a
// run with no shebang or interpreter dispatch; otherwise an inline snippet
// is TypeScript unless its shebang says otherwise.

package runner

import (
	"errors"
	"fmt"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/developing-today/code/pkgs/mcpx/internal/codegen"
)

// ScriptLanguage names the script language a run is written in, as an
// internal/codegen target name ("typescript", "python"). Callers use it to
// render the matching generated client; the runner uses the same answer to
// install it, so the two can never disagree about which client a run gets.
// An empty result means "no opinion": the run is TypeScript, as it has
// always been.
func ScriptLanguage(file, source string) string {
	first := firstLineOf(source)
	if first == "" && file != "" {
		// A read error is not fatal here: the run itself reports it with
		// better context when it opens the file for real.
		if l, err := fileFirstLine(file); err == nil {
			first = l
		}
	}
	if sp, ok := parseShebang(first); ok && len(sp.tokens) > 0 {
		if isPythonInterpreter(sp.tokens[0]) {
			return codegen.Python.Name
		}
		if isJSRuntimeName(Setup{}, sp.tokens[0]) {
			return codegen.TypeScript.Name
		}
	}
	switch strings.ToLower(filepath.Ext(file)) {
	case codegen.Python.ScriptExt:
		return codegen.Python.Name
	case ".ts", ".mts", ".js", ".mjs":
		return codegen.TypeScript.Name
	}
	return ""
}

// NormalizeLanguage validates an explicit script-language selection and
// returns the codegen language name it selects: "python" selects Python,
// "typescript"/"javascript" select TypeScript, and ""/"auto" select
// nothing (detection decides, as before). Any other value is an error the
// caller reports before a run starts, never a silent fallback.
func NormalizeLanguage(v string) (string, error) {
	switch strings.ToLower(strings.TrimSpace(v)) {
	case "", "auto":
		return "", nil
	case "python":
		return codegen.Python.Name, nil
	case "typescript", "javascript":
		return codegen.TypeScript.Name, nil
	default:
		return "", fmt.Errorf("runner: script language %q is not one of auto, typescript, javascript, python", v)
	}
}

// EffectiveScriptLanguage is ScriptLanguage with an explicit selection
// applied: a valid explicit language wins over shebang/extension
// detection; an empty, "auto" or invalid explicit value leaves detection
// in charge (validation of the explicit value is NormalizeLanguage's job,
// done once by the entry points).
func EffectiveScriptLanguage(file, source, explicit string) string {
	if n, err := NormalizeLanguage(explicit); err == nil && n != "" {
		return n
	}
	return ScriptLanguage(file, source)
}

// isPythonInterpreter reports whether a command names a Python interpreter:
// python, python3, a versioned python3.12, or uv (which runs a .py file in an
// environment built from the script's PEP 723 header).
func isPythonInterpreter(token string) bool {
	base := filepath.Base(token)
	if base == "uv" {
		return true
	}
	// filepath.Ext trims the version from python3.12, leaving python3;
	// python3.x is still Python, and the prefix test says so either way.
	return strings.HasPrefix(strings.TrimSuffix(base, filepath.Ext(base)), "python")
}

// resolvePython finds the interpreter a Python script runs under: python3,
// then the bare python some machines provide instead, then uv. uv is the
// fallback because it is the tool a Python author already has when python3 is
// missing (internal/hooks resolves a Python module the same way).
func resolvePython() (bin string, args []string, err error) {
	for _, candidate := range []string{"python3", "python"} {
		if p, lerr := exec.LookPath(candidate); lerr == nil {
			return p, nil, nil
		}
	}
	if p, lerr := exec.LookPath("uv"); lerr == nil {
		return p, []string{"run"}, nil
	}
	return "", nil, errors.New("runner: no python3, python or uv on PATH for a Python script")
}

// pythonLanguage is the codegen target a Python run renders and installs.
func pythonLanguage() codegen.Language {
	l, ok := codegen.Lookup(codegen.Python.Name)
	if !ok {
		// Registered by construction (codegen.Languages); a missing entry
		// is a programming error, and saying so beats installing a client
		// under the wrong name.
		return codegen.Python
	}
	return l
}

// pythonDispatch is the rank-3 dispatch: the file extension said Python and
// nothing named an interpreter, so the run uses the machine's Python.
func pythonDispatch(opts Options) (*dispatch, error) {
	bin, args, err := resolvePython()
	if err != nil {
		return nil, err
	}
	return &dispatch{
		bin:    bin,
		args:   args,
		name:   filepath.Base(bin),
		source: "language",
	}, nil
}

// harnessClient attaches the generated client for a dispatch's language, if
// this run has one. It is what turns a raw interpreter run into a harnessed
// one: the client is installed beside the script, the run is told where it
// is (MCPX_CLIENT), and the "no generated client" notice no longer applies.
func harnessClient(opts Options, d *dispatch) {
	if d == nil || !isPythonInterpreter(firstNonEmpty(d.lookupBin, d.bin)) {
		return
	}
	lang := codegen.Python.Name
	d.language = lang
	if c, ok := opts.Clients[lang]; ok {
		d.client = &c
	}
}

// firstNonEmpty returns the first non-empty string; it keeps the shebang
// lookup name (resolved at run time) ahead of the binary.
func firstNonEmpty(values ...string) string {
	for _, v := range values {
		if v != "" {
			return v
		}
	}
	return ""
}
