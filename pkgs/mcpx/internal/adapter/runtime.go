package adapter

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"github.com/dezren39/mcpx/internal/defaults"
)

// Runtime kinds a declaration may name. The set is the issue's: bash, js,
// ts, py. Each maps to a binary this machine has to provide.
const (
	RuntimeBash = "bash"
	RuntimeJS   = "js"
	RuntimeTS   = "ts"
	RuntimePy   = "py"
)

// runtimeBinaries is the interpreter each kind resolves through, in the
// order tried. bash and py have one obvious answer plus the fallback name a
// machine without it actually uses; JavaScript reuses defaults.RuntimeOrder,
// the same deno/bun/node preference `mcpx exec` applies, because two orders
// for the same three binaries would be a preference nobody could predict.
var runtimeBinaries = map[string][]string{
	RuntimeBash: {"bash", "sh"},
	RuntimePy:   {"python3", "python"},
	RuntimeJS:   defaults.RuntimeOrder,
	RuntimeTS:   defaults.RuntimeOrder,
}

// runtimePrefix is argv placed between the interpreter and the script.
//
// Only deno needs one: `deno script.ts` is not a thing, `deno run
// script.ts` is. bun and node take the file directly.
var runtimePrefix = map[string][]string{
	"deno": {"run"},
}

// interpreterName is a binary's bare name, without a Windows extension, so
// the deno `run` prefix is found however the path is spelled.
func interpreterName(path string) string {
	base := filepath.Base(path)
	return strings.TrimSuffix(base, filepath.Ext(base))
}

// checkRuntime rejects an unknown runtime name with the list of the ones
// that exist. Kept separate from resolution so Validate does not depend on
// what is installed.
func checkRuntime(name string) error {
	if _, ok := runtimeBinaries[strings.ToLower(name)]; ok {
		return nil
	}
	return fmt.Errorf("runtime %q is not supported; want bash|js|ts|py", name)
}

// RuntimeInterpreter resolves a runtime name to a binary and the argv that
// must precede the script.
//
// A declaration naming bash or py is asking this machine for its
// interpreter, the same way a command name asks for a binary on PATH. Which
// binary answered is returned, so a caller can report it rather than let the
// author guess what "py" ran.
func RuntimeInterpreter(name string) (bin string, prefix []string, err error) {
	kind := strings.ToLower(strings.TrimSpace(name))
	if err := checkRuntime(kind); err != nil {
		return "", nil, err
	}
	var tried []string
	for _, candidate := range runtimeBinaries[kind] {
		p, lerr := exec.LookPath(candidate)
		if lerr != nil {
			tried = append(tried, candidate)
			continue
		}
		return p, append([]string{}, runtimePrefix[interpreterName(p)]...), nil
	}
	return "", nil, fmt.Errorf("runtime %q: none of %s is on PATH",
		name, strings.Join(tried, ", "))
}

// command resolves how this declaration is spawned.
//
// Without a runtime this is exactly what it always was: Command on PATH.
// With one, Command is a script -- a path to the file, or a name on PATH --
// and the runtime supplies the executable. Command itself stays what the
// declaration wrote, which is what listings show; the interpreter and script
// are what actually run.
func (s Spec) command() (bin string, prefix []string, script string, err error) {
	if s.Runtime == "" {
		p, lerr := exec.LookPath(s.Command)
		if lerr != nil {
			return "", nil, "", fmt.Errorf("%s is not on PATH", s.Command)
		}
		return p, nil, "", nil
	}
	bin, prefix, err = RuntimeInterpreter(s.Runtime)
	if err != nil {
		return "", nil, "", err
	}
	command := s.Command
	if s.Dir != "" && !filepath.IsAbs(command) {
		// The declaration already says where the program's files live, and
		// a script written beside them is the ordinary case.
		command = filepath.Join(s.Dir, command)
	}
	script, err = scriptPath(command)
	if err != nil {
		return "", nil, "", err
	}
	return bin, prefix, script, nil
}

// scriptPath finds the file a runtime should execute.
//
// An absolute path, a path relative to the declaration's Dir, or a name on
// PATH: all three are what people mean by "point it at this script". A
// command that is neither an existing file nor on PATH is the error worth
// having, because it is the one that would otherwise surface as an
// interpreter complaining about a file nobody named.
func scriptPath(command string) (string, error) {
	if command == "" {
		return "", fmt.Errorf("no script to run")
	}
	if filepath.IsAbs(command) {
		if _, err := os.Stat(command); err != nil {
			return "", fmt.Errorf("%s is not a file: %w", command, err)
		}
		return command, nil
	}
	if _, err := os.Stat(command); err == nil {
		abs, aerr := filepath.Abs(command)
		if aerr != nil {
			return command, nil
		}
		return abs, nil
	}
	if p, err := exec.LookPath(command); err == nil {
		return p, nil
	}
	return "", fmt.Errorf("%s is not a script file and is not on PATH", command)
}
