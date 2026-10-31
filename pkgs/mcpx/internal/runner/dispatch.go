// Interpreter dispatch for non-TypeScript scripts (#2167, parts 1-3 of
// #1919).
//
// Dispatch order, first match wins:
//
//  1. The source or file starts with a shebang (#!): that interpreter,
//     verbatim. A shebang is an instruction inside the program itself, so
//     it beats every configuration surface -- including script.runtime:
//     a file that names its own interpreter is not overridden by a
//     machine-wide preference for a different one. One exception: a
//     shebang naming a JavaScript runtime (deno, bun, node, or a
//     declared runtime) with no extra arguments goes through the
//     harness with that runtime, because raw dispatch would skip the
//     generated launcher and the script's default export would never
//     be called.
//  2. script.interpreter: a configured command line, with the script
//     path appended.
//  3. script.language / file extension: RESERVED for #1917. Nothing
//     diverts here yet, by design -- the rank exists so the order does
//     not change when it lands.
//  4. The configured TypeScript runtime: today's behaviour, unchanged
//     when everything above is unset.
//
// A dispatched (non-JavaScript) run gets the harness posture, not the
// harness: MCPX_ENDPOINT / MCPX_SESSION / MCPX_SOCKET arrive through
// Options.Env exactly as they do for a TypeScript run, MCPX_RUNTIME
// names the interpreter, MCPX_ENTRY names the script, the caller's args
// follow the script path, and stdout/stderr/exit/timeout handling is the
// same machinery. What it does not get is the generated client -- there
// is no TypeScript module a shell script could import -- and that
// absence is stated once per run (Result.Notice, and a line on stderr),
// not left for the script to discover. script.permissions profiles are
// JavaScript-runtime flags; no shell interpreter can enforce them, so
// they are not applied to a dispatched run and the notice says so.
package runner

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"time"

	"github.com/dezren39/mcpx/internal/defaults"
	"github.com/dezren39/mcpx/internal/logging"
)

// CwdPolicies are the script.cwd values, in the order the setting
// documents them.
var CwdPolicies = []string{"caller", "script", "config", "workdir"}

// ResolveCwd applies a script.cwd policy. callerDir is Options.Dir (the
// caller's explicit directory, empty meaning inherit), scriptPath the
// final entry script, configDir the directory holding the mcpx
// configuration in force, and workDir the runner's working directory
// for the run (where a snippet and the generated client live).
func ResolveCwd(policy, callerDir, scriptPath, configDir, workDir string) (string, error) {
	switch policy {
	case "", "caller":
		return callerDir, nil
	case "script":
		if scriptPath == "" {
			return "", errors.New("script.cwd=script but the run has no script file")
		}
		return filepath.Dir(scriptPath), nil
	case "config":
		if configDir == "" {
			return "", errors.New("script.cwd=config but no configuration directory is known for this run")
		}
		return configDir, nil
	case "workdir":
		if workDir == "" {
			return "", errors.New("script.cwd=workdir but the run has no working directory")
		}
		return workDir, nil
	}
	return "", fmt.Errorf("script.cwd %q is not one of %s", policy, strings.Join(CwdPolicies, ", "))
}

// dispatch is a resolved non-JavaScript interpreter invocation: the
// binary, the arguments that precede the script path, and where the
// decision came from.
type dispatch struct {
	bin    string
	args   []string
	env    []string // VAR=value assignments an env shebang carried
	unset  []string // names an env shebang removed from the environment (-u)
	clear  bool     // an env shebang asked for an empty environment (-i)
	chdir  string   // directory an env shebang asked to run in (-C)
	name   string   // interpreter name reported as the run's runtime
	source string   // "shebang" or "interpreter"
	// lookupBin/lookupPath defer a shebang's interpreter lookup to
	// run time: relative PATH entries resolve against the run's
	// effective directory (policy + the shebang's -C), the way
	// execvp resolves them after env(1) has applied -C. Resolving
	// at decide time would check them against mcpx's own working
	// directory, which is the wrong directory whenever the run
	// moves elsewhere (#2611 review).
	lookupBin  string // command name as the shebang named it (no "/")
	lookupPath string // PATH value the shebang establishes
}

// ParseShebang splits a #! line into the interpreter command it names.
// The /usr/bin/env form is unwrapped the way the kernel cannot: env's
// own options are consumed, and what remains is the command. ok is
// false when the line is not a shebang.
func ParseShebang(line string) (tokens []string, ok bool) {
	sp, ok := parseShebang(line)
	return sp.tokens, ok
}

// ParseShebangEnv is ParseShebang that also returns the VAR=value
// assignments an env shebang carried (#!/usr/bin/env MODE=prod sh).
// env(1) would set them for the command it runs; dropping them would
// run the script under a different environment than its own first line
// specifies, silently, so the dispatch carries them into the child
// environment instead.
func ParseShebangEnv(line string) (tokens []string, env []string, ok bool) {
	sp, ok := parseShebang(line)
	return sp.tokens, sp.set, ok
}

// shebangEnv is a parsed shebang: the command it names, plus the
// environment changes an env(1) shebang asked for and the kernel never
// got to apply.
type shebangEnv struct {
	tokens []string
	set    []string // VAR=value assignments
	unset  []string // names removed from the inherited environment (-u)
	clear  bool     // start from an empty environment (-i)
	chdir  string   // directory env(1) would have moved to (-C/--chdir)
	// splitErr is why an env -S payload could not be re-split (an
	// unterminated quote). It is reported by decideDispatch; the
	// exported ParseShebang wrappers keep best-effort tokens.
	splitErr error
	// optErr is an env(1) option the shebang used that this dispatcher
	// does not implement. Reported by decideDispatch, which refuses the
	// run rather than dropping the option (#2611 review).
	optErr error
}

// parseShebang reads a #! line. Options of an env(1) shebang are
// consumed as env itself would consume them, including the operand of
// -u NAME and -C DIR, so that what remains is the command: without
// that, "#!/usr/bin/env -S -u SECRET sh" would try to run SECRET.
//
// A -S/--split-string payload is re-split with shell quoting rules,
// because the kernel hands env the line already flattened: without
// that, `#!/usr/bin/env -S sh -c "printf hello"` would run sh with
// the two words '"printf' and 'hello"' and fail on the quotes. The
// payload reaches the splitter UNTOUCHED (the raw remainder of the
// line after the -S token), not re-joined from pre-split fields:
// fieldsplitting first discards whitespace inside quotes, so
// `sh -c 'printf "<%s>" "a  b"'` would silently run with "a b"
// (#2611 review).
//
// "--" ends option processing but not assignment processing: env's
// grammar is [OPTION]... [NAME=VALUE]... COMMAND, and assignments
// after the terminator still apply, so "#!/usr/bin/env -- MODE=prod
// sh" runs sh with MODE=prod set instead of trying to execute a
// program named MODE=prod (#2611 review).
func parseShebang(line string) (shebangEnv, bool) {
	line = strings.TrimRight(line, "\r\n")
	if !strings.HasPrefix(line, "#!") {
		return shebangEnv{}, false
	}
	rest := strings.TrimSpace(line[2:])
	if rest == "" {
		return shebangEnv{}, true
	}
	fields := strings.Fields(rest)
	var sp shebangEnv
	if filepath.Base(fields[0]) != "env" {
		sp.tokens = fields
		return sp, true
	}
	// env shebang: consume tokens from the raw remainder of the line,
	// so a -S payload can still be recovered verbatim; tokens produced
	// by a re-split queue in pending instead.
	raw := strings.TrimSpace(rest[len(fields[0]):])
	var pending []string
	next := func() (string, bool) {
		if len(pending) > 0 {
			t := pending[0]
			pending = pending[1:]
			return t, true
		}
		raw = strings.TrimLeft(raw, " \t")
		if raw == "" {
			return "", false
		}
		if i := strings.IndexAny(raw, " \t"); i >= 0 {
			t := raw[:i]
			raw = raw[i:]
			return t, true
		}
		t := raw
		raw = ""
		return t, true
	}
	unread := func(t string) { pending = append([]string{t}, pending...) }
	peek := func() (string, bool) {
		t, ok := next()
		if ok {
			unread(t)
		}
		return t, ok
	}
	// remaining returns first plus everything not yet consumed, as
	// the command and its arguments.
	remaining := func(first string) []string {
		out := []string{first}
		out = append(out, pending...)
		if strings.TrimSpace(raw) != "" {
			out = append(out, strings.Fields(raw)...)
		}
		return out
	}
	// payloadFor builds a -S payload: with nothing queued the raw
	// remainder is the payload, verbatim; with tokens queued by an
	// earlier re-split it falls back to joining what remains.
	payloadFor := func(head string) string {
		if len(pending) == 0 {
			tail := strings.TrimLeft(raw, " \t")
			raw = ""
			if head == "" {
				return tail
			}
			if tail == "" {
				return head
			}
			return head + " " + tail
		}
		parts := append([]string{}, pending...)
		if strings.TrimSpace(raw) != "" {
			parts = append(parts, strings.Fields(raw)...)
		}
		pending = nil
		raw = ""
		if head == "" {
			return strings.Join(parts, " ")
		}
		return head + " " + strings.Join(parts, " ")
	}
	for {
		f, ok := next()
		if !ok {
			return sp, true
		}
		switch {
		case f == "-i" || f == "--ignore-environment" || f == "-":
			sp.clear = true
		case f == "--":
			// Options are over; NAME=VALUE assignments are not.
			for {
				t, ok := peek()
				if !ok || strings.HasPrefix(t, "-") || !strings.Contains(t, "=") {
					break
				}
				next()
				sp.set = append(sp.set, t)
			}
			if t, ok := next(); ok {
				sp.tokens = remaining(t)
			}
			return sp, true
		case f == "-u" || f == "--unset" || f == "-C" || f == "--chdir":
			operand, ok := next()
			if !ok {
				return sp, true
			}
			if f == "-u" || f == "--unset" {
				sp.unset = append(sp.unset, operand)
			} else {
				sp.chdir = operand
			}
		case strings.HasPrefix(f, "--unset="):
			sp.unset = append(sp.unset, strings.TrimPrefix(f, "--unset="))
		case strings.HasPrefix(f, "-u") && len(f) > 2:
			sp.unset = append(sp.unset, f[2:])
		case f == "-S" || f == "--split-string":
			pending = sp.resplit(payloadFor(""))
		case strings.HasPrefix(f, "--split-string="):
			pending = sp.resplit(payloadFor(strings.TrimPrefix(f, "--split-string=")))
		case strings.HasPrefix(f, "-S") && len(f) > 2:
			pending = sp.resplit(payloadFor(f[2:]))
		case strings.HasPrefix(f, "-C") && len(f) > 2:
			sp.chdir = f[2:]
		case strings.HasPrefix(f, "--chdir="):
			sp.chdir = strings.TrimPrefix(f, "--chdir=")
		case strings.HasPrefix(f, "-"):
			// Anything left is an env(1) option this dispatcher cannot
			// honour. Silently dropping it would run a different command
			// than the shebang names -- `#!/usr/bin/env -S --bogus sh`
			// would run sh with the option ignored -- and the options
			// that would be dropped are not all inert: -0 makes env
			// reject a command outright, -v/--debug write their own
			// output to stderr (#2611 review).
			sp.optErr = fmt.Errorf("%s is not a supported env option in a shebang", f)
			return sp, true
		case strings.Contains(f, "="):
			sp.set = append(sp.set, f)
		default:
			sp.tokens = remaining(f)
			return sp, true
		}
	}
}

// resplit parses an env -S payload the way env(1) would. A payload
// that does not parse records the error for decideDispatch and
// yields no tokens.
func (sp *shebangEnv) resplit(payload string) []string {
	fields, err := SplitEnvString(payload)
	if err != nil {
		sp.splitErr = err
		return nil
	}
	return fields
}

// SplitEnvString splits an env(1) -S/--split-string payload with
// env's own semantics, which are NOT the shell's: env defines a
// backslash escape table (\\, \", \', \t, \n, \r, \f, \v and
// \_) that applies inside quotes as well as outside --
// the coreutils manual's own example, -S awk -v FS=\"\\t\" -f,
// depends on \\t becoming a tab, where a shell tokenizer leaves a
// literal "t" (Codex review, PR #2611). Quotes still group, as the
// earlier rounds of this parser established. Two rules are easy to
// get wrong, both verified against env(1) itself: \_ outside quotes
// is an argument SEPARATOR, not a space (inside double quotes it is
// a literal space), and ${VAR} expands from the INHERITED
// environment -- env expands before it applies the payload's own
// VAR=value assignments, so `MODE=prod ... ${MODE}` sees the
// inherited MODE, never "prod" (Codex review, PR #2611). An
// unquoted # that begins an argument starts a comment: the rest of
// the payload is ignored.
func SplitEnvString(s string) ([]string, error) {
	var out []string
	var word strings.Builder
	quoted := byte(0)
	started := false
	flush := func() {
		if !started {
			return
		}
		out = append(out, word.String())
		word.Reset()
		started = false
	}
	expand := func(i int) (string, int, error) {
		// s[i] == '$' and s[i+1] == '{'.
		j := strings.IndexByte(s[i:], '}')
		if j < 0 {
			return "", 0, fmt.Errorf("unterminated ${ in env -S payload %q", s)
		}
		name := s[i+2 : i+j]
		return os.Getenv(name), i + j + 1, nil
	}
	for i := 0; i < len(s); i++ {
		c := s[i]
		switch {
		case c == '\\' && quoted == '\'':
			// env's manual: inside single quotes only \' and \\ are
			// escapes; every other backslash stays literal. Decoding
			// them turned python3 -c '…re.search(r"\d", "5")…' into
			// r"d" and changed the program's answer (#2611 review).
			if i+1 < len(s) && (s[i+1] == '\'' || s[i+1] == '\\') {
				i++
				word.WriteByte(s[i])
			} else {
				word.WriteByte(c)
			}
			started = true
		case c == '\\':
			if i+1 >= len(s) {
				return nil, fmt.Errorf("trailing backslash in env -S payload %q", s)
			}
			i++
			switch s[i] {
			case 't':
				word.WriteByte('\t')
			case 'n':
				word.WriteByte('\n')
			case 'r':
				word.WriteByte('\r')
			case 'f':
				word.WriteByte('\f')
			case 'v':
				word.WriteByte('\v')
			case '_':
				// Outside quotes \_ is an argument separator
				// (env(1): `python3\_-u` is two arguments);
				// inside double quotes it is a literal space.
				// The single-quote case never reaches here.
				if quoted == '"' {
					word.WriteByte(' ')
					started = true
				} else {
					flush()
				}
				continue
			default:
				word.WriteByte(s[i])
			}
			started = true
		case quoted != 0:
			if c == quoted {
				quoted = 0
			} else if c == '$' && quoted == '"' && i+1 < len(s) && s[i+1] == '{' {
				v, ni, err := expand(i)
				if err != nil {
					return nil, err
				}
				word.WriteString(v)
				i = ni - 1
			} else {
				word.WriteByte(c)
			}
			started = true
		case c == '\'' || c == '"':
			quoted = c
			started = true
		case c == '$' && i+1 < len(s) && s[i+1] == '{':
			v, ni, err := expand(i)
			if err != nil {
				return nil, err
			}
			word.WriteString(v)
			i = ni - 1
			started = true
		case c == '#' && !started:
			// An unquoted # that begins an argument starts a
			// comment (env(1)): the rest of the payload is not
			// arguments. A # inside an argument stays literal,
			// and quoted or escaped hashes never reach here.
			flush()
			return out, nil
		case c == ' ' || c == '\t':
			flush()
		default:
			word.WriteByte(c)
			started = true
		}
	}
	if quoted != 0 {
		return nil, fmt.Errorf("unterminated %q in env -S payload %q", string(quoted), s)
	}
	flush()
	return out, nil
}

// filterEnv drops the named variables from env, leaving everything else
// in order. The result is a fresh slice; env is not modified.
func filterEnv(env []string, unset []string) []string {
	if len(unset) == 0 {
		return env
	}
	drop := make(map[string]bool, len(unset))
	for _, name := range unset {
		drop[name] = true
	}
	out := make([]string, 0, len(env))
	for _, kv := range env {
		name, _, _ := strings.Cut(kv, "=")
		if !drop[name] {
			out = append(out, kv)
		}
	}
	return out
}

// isJSRuntimeName reports whether a shebang's command names a
// JavaScript runtime mcpx can drive through the harness: a known kind
// (deno, bun, node), by name or by path, or a name declared under
// script.runtimes.
func isJSRuntimeName(setup Setup, name string) bool {
	if _, ok := setup.Runtimes[name]; ok {
		return true
	}
	base := filepath.Base(name)
	if _, ok := setup.Runtimes[base]; ok {
		return true
	}
	stem := strings.TrimSuffix(base, filepath.Ext(base))
	return knownKind(strings.ToLower(stem))
}

// SplitCommandLine splits a configured command line (script.interpreter)
// into words, honouring single and double quotes and backslash escapes.
// It is deliberately small: an interpreter command line is a handful of
// words, and anything fancier belongs in a script of its own.
func SplitCommandLine(s string) ([]string, error) {
	var out []string
	var word strings.Builder
	quoted := byte(0)
	escaped := false
	started := false
	flush := func() {
		if started {
			out = append(out, word.String())
			word.Reset()
			started = false
		}
	}
	for i := 0; i < len(s); i++ {
		c := s[i]
		switch {
		case escaped:
			word.WriteByte(c)
			escaped = false
			started = true
		case c == '\\' && quoted == 0:
			escaped = true
		case c == '\\' && quoted == '"':
			// Inside double quotes the shell keeps a backslash literal
			// unless it precedes one of the characters it still escapes
			// ($ ` " \ and newline). Consuming every backslash turned
			// `awk -v FS="\t" -f` into `FS=t`, splitting on the letter t
			// (#2611 review).
			if i+1 < len(s) && strings.ContainsRune("$`\"\\\n", rune(s[i+1])) {
				escaped = true
			} else {
				word.WriteByte(c)
				started = true
			}
		case quoted != 0:
			if c == quoted {
				quoted = 0
			} else {
				word.WriteByte(c)
			}
			started = true
		case c == '\'' || c == '"':
			quoted = c
			started = true
		case c == ' ' || c == '\t':
			flush()
		default:
			word.WriteByte(c)
			started = true
		}
	}
	if quoted != 0 {
		return nil, fmt.Errorf("unterminated %q in command line %q", string(quoted), s)
	}
	flush()
	return out, nil
}

func firstLineOf(s string) string {
	if i := strings.IndexByte(s, '\n'); i >= 0 {
		return s[:i]
	}
	return s
}

func fileFirstLine(path string) (string, error) {
	f, err := os.Open(path)
	if err != nil {
		return "", err
	}
	defer f.Close()
	line, err := bufio.NewReader(io.LimitReader(f, 8192)).ReadString('\n')
	if err != nil && line == "" {
		return "", err
	}
	return line, nil
}

// jsChoice is a shebang's choice of JavaScript runtime (rank 1): the
// runtime the harness must use, plus the environment changes the
// env(1) wrapper around it asked for. The environment rides with the
// runtime so the harness path cannot silently drop what
// ParseShebangEnv preserves for the raw path (#2611 review).
type jsChoice struct {
	runtime string
	set     []string
	unset   []string
	clear   bool
	chdir   string
	// path is the PATH env(1) would resolve the runtime against
	// (shebangPath of the parsed wrapper), carried so the harness
	// resolves the runtime against the shebang's environment and
	// effective directory instead of mcpx's inherited PATH (Codex
	// review, PR #2611).
	path string
}

// decideDispatch applies ranks 1 and 2 of the dispatch order. A nil
// dispatch means the run falls through to the TypeScript runtime; the
// second return value then carries a JavaScript shebang's runtime
// choice, so the harness honours the shebang's choice instead of the
// configured preference.
//
// A shebang naming a JavaScript runtime, with no extra arguments, is
// that exception: raw dispatch would bypass the generated launcher, so
// a script whose default export is the program would run without its
// entry point ever being called (the pkgs/mcpx/AGENTS.md script
// contract). The harness path below keeps the contract and takes the
// shebang's runtime as its preference. A JavaScript shebang WITH extra
// arguments keeps raw dispatch: the arguments are part of the command
// the file names, and the harness has nowhere to put them.
func decideDispatch(opts Options) (*dispatch, jsChoice, error) {
	first := ""
	if opts.Source != "" {
		first = firstLineOf(opts.Source)
	} else if opts.File != "" {
		// A read error here is not fatal to the decision: the normal
		// path surfaces it with better context when it opens the file
		// for real. Only a shebang that IS readable diverts the run.
		if l, err := fileFirstLine(opts.File); err == nil {
			first = l
		}
	}
	if sp, ok := parseShebang(first); ok {
		if sp.splitErr != nil {
			return nil, jsChoice{}, fmt.Errorf("runner: shebang -S payload: %w", sp.splitErr)
		}
		if sp.optErr != nil {
			return nil, jsChoice{}, fmt.Errorf("runner: shebang: %w", sp.optErr)
		}
		if len(sp.tokens) == 0 {
			return nil, jsChoice{}, errors.New("runner: the script's shebang line (#!) names no interpreter")
		}
		if len(sp.tokens) == 1 && isJSRuntimeName(opts.Setup, sp.tokens[0]) {
			return nil, jsChoice{runtime: sp.tokens[0], set: sp.set, unset: sp.unset, clear: sp.clear, chdir: sp.chdir, path: shebangPath(&sp)}, nil
		}
		d, err := dispatchFromTokens("shebang", sp.tokens, &sp)
		if err != nil {
			return nil, jsChoice{}, err
		}
		d.env, d.unset, d.clear, d.chdir = sp.set, sp.unset, sp.clear, sp.chdir
		return d, jsChoice{}, nil
	}
	if strings.TrimSpace(opts.Interpreter) != "" {
		tokens, err := SplitCommandLine(opts.Interpreter)
		if err != nil {
			return nil, jsChoice{}, fmt.Errorf("runner: script.interpreter: %w", err)
		}
		if len(tokens) == 0 {
			return nil, jsChoice{}, errors.New("runner: script.interpreter is set but names no command")
		}
		d, err := dispatchFromTokens("interpreter", tokens, nil)
		if err != nil {
			return nil, jsChoice{}, err
		}
		return d, jsChoice{}, nil
	}
	return nil, jsChoice{}, nil
}

// applyChdir applies an env shebang's -C/--chdir directory to the
// policy directory a run resolved. A shebang's -C is the program's own
// directory choice, so like its interpreter choice it beats
// configuration: the rank-1 rationale in decideDispatch applies to the
// directory too. A relative -C resolves against the policy directory,
// the way env(1) resolves it against the directory in force. Both
// dispatch paths (raw and the JavaScript harness) apply it through
// here, so neither can silently drop what the shebang asked for.
func applyChdir(dir, chdir string) string {
	if chdir == "" {
		return dir
	}
	if filepath.IsAbs(chdir) {
		return chdir
	}
	return filepath.Join(dir, chdir)
}

// ownedEnvNames are the MCPX_* variables a run sets itself. A child
// never inherits them: with duplicate keys the first wins, so a stale
// value from an outer run (a MCPX_CLIENT path, a MCPX_RUNTIME naming
// another interpreter) would silently beat the run's own contract.
var ownedEnvNames = []string{"MCPX_CLIENT", "MCPX_RUNTIME", "MCPX_ENTRY", "MCPX_DISPATCH"}

// childBaseEnv is the environment a diverted run starts from: the
// inherited environment (or nothing, with -i), minus the names env(1)
// was told to unset and the run's own contract variables, plus the
// assignments the shebang carried.
func childBaseEnv(set, unset []string, clear bool) []string {
	var base []string
	if !clear {
		drop := make([]string, 0, len(unset)+len(ownedEnvNames))
		drop = append(drop, unset...)
		drop = append(drop, ownedEnvNames...)
		base = filterEnv(os.Environ(), drop)
	}
	return append(base, set...)
}

// shebangPath is the PATH env(1) would run the command with: the
// inherited one, emptied by -i or by unsetting PATH, with the shebang's
// own PATH assignment (the last one) replacing it. Resolving the
// interpreter against anything else runs a different interpreter than
// the one the script's first line names -- or none, when the executable
// exists only in the assigned PATH (#2611 review).
//
// Absence and emptiness are different states and are tracked as such:
// an ABSENT PATH (cleared with -i, unset with -u, or never inherited)
// gets execvp's default (/bin:/usr/bin), while an explicitly EMPTY
// PATH (PATH= in the shebang) stays empty, which execvp reads as
// "search the effective directory only" -- substituting the default
// for it would run a system executable the shebang did not name, or
// miss the one in the run's directory (#2611 review).
func shebangPath(sp *shebangEnv) string {
	path, present := os.LookupEnv("PATH")
	if sp.clear {
		path, present = "", false
	}
	for _, u := range sp.unset {
		if u == "PATH" {
			path, present = "", false
		}
	}
	for _, kv := range sp.set {
		if k, v, ok := strings.Cut(kv, "="); ok && k == "PATH" {
			path, present = v, true
		}
	}
	if !present {
		// env(1) runs the command through execvp, which falls back to
		// a default path (confstr _CS_PATH: /bin:/usr/bin) when PATH is
		// absent from the environment it builds -- the -i case.
		return "/bin:/usr/bin"
	}
	return path
}

// lookPathInDir is exec.LookPath against one explicit PATH value,
// with entries resolved against dir: an empty PATH value means the
// effective directory alone, and an empty entry means the run's
// effective directory (not mcpx's own), and so does any other relative
// entry, the way execvp resolves them after env(1) has applied -C. The
// result is absolute, so the later exec cannot reinterpret it against
// a third directory.
func lookPathInDir(bin, path, dir string) (string, error) {
	abs, err := filepath.Abs(dir)
	if err != nil {
		return "", err
	}
	entries := filepath.SplitList(path)
	if path == "" {
		// An explicitly empty PATH is a single empty entry: the
		// effective directory, and nothing else.
		entries = []string{""}
	}
	for _, entry := range entries {
		if entry == "" {
			entry = abs
		} else if !filepath.IsAbs(entry) {
			entry = filepath.Join(abs, entry)
		}
		p := filepath.Join(entry, bin)
		if fi, err := os.Stat(p); err == nil && !fi.IsDir() && fi.Mode()&0o111 != 0 {
			return p, nil
		}
	}
	return "", exec.ErrNotFound
}

func dispatchFromTokens(source string, tokens []string, sp *shebangEnv) (*dispatch, error) {
	bin := tokens[0]
	d := &dispatch{
		args:   append([]string{}, tokens[1:]...),
		name:   filepath.Base(bin),
		source: source,
	}
	if strings.Contains(bin, "/") {
		// An explicit path is used verbatim, the way the kernel
		// uses a slash-containing shebang path as-is.
		d.bin = bin
		return d, nil
	}
	if sp != nil {
		// A shebang's command is resolved against the environment
		// the shebang itself establishes, the way env(1) resolves
		// it after applying its assignments -- but only once the
		// run's effective directory is known (runDispatched): its
		// relative PATH entries mean the directory the run moves
		// to, not mcpx's own.
		d.lookupBin, d.lookupPath = bin, shebangPath(sp)
		return d, nil
	}
	// Always deferred, found or not: a relative PATH entry means the
	// directory the run moves to, not mcpx's own, so a lookup here can
	// both miss the right binary and find a different one. runDispatched
	// resolves against the run's effective directory and reports the
	// not-found error with the PATH it tried (#2611 review). A PATH that
	// is absent from the environment keeps exec.LookPath's answer, since
	// there is nothing to defer to.
	if path, ok := os.LookupEnv("PATH"); ok {
		d.lookupBin, d.lookupPath = bin, path
		return d, nil
	}
	p, err := exec.LookPath(bin)
	if err != nil {
		return nil, fmt.Errorf("runner: %s interpreter %q not found on PATH: %w", source, bin, err)
	}
	d.bin = p
	return d, nil
}

// runDispatched executes a script under a non-JavaScript interpreter.
// The process machinery mirrors Run's: own process group, structured
// stderr streaming when a log writer is set, SIGTERM-then-SIGKILL on
// timeout, exit codes reported rather than errored.
func runDispatched(ctx context.Context, opts Options, d *dispatch) (*Result, error) {
	scriptPath := opts.File
	workDir := opts.WorkDir
	cleanup := func() {}
	if scriptPath != "" {
		abs, err := filepath.Abs(scriptPath)
		if err != nil {
			return nil, err
		}
		scriptPath = abs
		if workDir == "" && opts.WorkRoot != "" {
			// A file run has no generated client to key on, so its
			// runner directory under the root is keyed on the
			// script itself. Allocating it
			// here is what makes script.cwd=workdir mean the
			// runner directory for a dispatched run, the way it
			// does for a JavaScript run; substituting the script's
			// own directory (the old fallback below) made workdir
			// behave identically to script, and relative output
			// landed beside the source (Codex review, PR #2611).
			sum := sha256sum([]byte(scriptPath))
			workDir = filepath.Join(opts.WorkRoot, sum[:16])
			if err := os.MkdirAll(workDir, 0o700); err != nil {
				return nil, err
			}
			pruneWorkDirs(opts.WorkRoot, workDir)
		}
	} else {
		// A snippet runs from a file of its own: no Prelude (that is
		// TypeScript for the generated client) and no .ts suffix (the
		// interpreter, not the name, decides what it is).
		if workDir == "" {
			wd, err := os.MkdirTemp("", "mcpx-run-")
			if err != nil {
				return nil, err
			}
			workDir = wd
			cleanup = func() { _ = os.RemoveAll(wd) }
		} else if err := os.MkdirAll(workDir, 0o700); err != nil {
			return nil, err
		}
		sum := sha256sum([]byte(opts.Source))
		scriptPath = filepath.Join(workDir, "script-"+sum[:16])
		if err := writeIfChanged(scriptPath, opts.Source); err != nil {
			cleanup()
			return nil, err
		}
	}
	defer cleanup()
	// No script-directory fallback for workDir: with no WorkDir and no
	// WorkRoot, a script.cwd=workdir run has no runner directory, and
	// ResolveCwd says so explicitly rather than silently running in
	// the script's directory under the workdir name.

	dir, err := ResolveCwd(opts.CwdPolicy, opts.Dir, scriptPath, opts.ConfigDir, workDir)
	if err != nil {
		return nil, err
	}
	dir = applyChdir(dir, d.chdir)
	if d.lookupBin != "" {
		// The interpreter a shebang names is resolved here, against
		// the directory the run actually moves to: deciding earlier
		// would check relative PATH entries against mcpx's own
		// working directory and miss an interpreter that exists
		// only where the run goes.
		abs, err := lookPathInDir(d.lookupBin, d.lookupPath, dir)
		if err != nil {
			return nil, fmt.Errorf("runner: %s interpreter %q not found on PATH: %w", d.source, d.lookupBin, err)
		}
		d.bin = abs
	}

	notice := fmt.Sprintf("mcpx: running under %s (%s dispatch): no generated mcpx client is "+
		"available to a non-JavaScript script, and script.permissions profiles are not enforced "+
		"for this interpreter; MCPX_ENDPOINT, MCPX_SESSION and MCPX_SOCKET are in the environment",
		d.name, d.source)
	if !opts.Phases.Empty() {
		notice += "; configured launcher phases do not apply to a non-JavaScript run and are skipped"
	}

	runCtx := ctx
	var cancel context.CancelFunc
	if opts.Timeout > 0 {
		runCtx, cancel = context.WithTimeout(ctx, opts.Timeout)
		defer cancel()
	}

	args := append(append([]string{}, d.args...), scriptPath)
	args = append(args, opts.Args...)
	cmd := exec.Command(d.bin, args...)
	cmd.Dir = dir
	// The shebang's own environment comes first of the additions: env(1)
	// would have started from the inherited environment (or an empty one,
	// with -i) minus the names it was told to unset, set its assignments,
	// and then the MCPX_* variables win any collision, as the run's own
	// contract should. Names the run owns are never inherited: a stale
	// outer value would shadow the contract (first duplicate wins).
	cmd.Env = childBaseEnv(d.env, d.unset, d.clear)
	cmd.Env = append(cmd.Env, "MCPX_RUNTIME="+d.name, "MCPX_ENTRY="+scriptPath, "MCPX_DISPATCH="+d.source)
	for k, v := range opts.Env {
		cmd.Env = append(cmd.Env, k+"="+v)
	}
	if opts.Stdout != nil {
		cmd.Stdout = opts.Stdout
	} else {
		cmd.Stdout = os.Stdout
	}
	var stderrDone chan struct{}
	if opts.Log != nil {
		pr, pw, perr := os.Pipe()
		if perr != nil {
			return nil, perr
		}
		cmd.Stderr = pw
		passthrough := opts.Stderr
		if passthrough == nil {
			passthrough = os.Stderr
		}
		stderrDone = make(chan struct{})
		go func() {
			defer close(stderrDone)
			defer pr.Close()
			_ = logging.Stream(pr, opts.Log, logging.StreamOptions{
				Enrich:      opts.Enrich,
				Passthrough: passthrough,
				Collect:     opts.CollectLogs,
				Result:      opts.OnResult,
				Artifact:    opts.OnArtifact,
			})
		}()
		// The notice rides the same stream as the script's own stderr,
		// ahead of anything the script prints.
		fmt.Fprintln(pw, notice)
		defer func() {
			pw.Close()
			<-stderrDone
		}()
	} else if opts.Stderr != nil {
		cmd.Stderr = opts.Stderr
		fmt.Fprintln(opts.Stderr, notice)
	} else {
		cmd.Stderr = os.Stderr
		fmt.Fprintln(os.Stderr, notice)
	}
	if opts.Stdin != nil {
		cmd.Stdin = opts.Stdin
	} else {
		cmd.Stdin = os.Stdin
	}
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}

	start := time.Now()
	if err := cmd.Start(); err != nil {
		return nil, fmt.Errorf("start %s: %w", d.name, err)
	}
	waitCh := make(chan error, 1)
	go func() { waitCh <- cmd.Wait() }()

	res := &Result{Runtime: d.name, Script: scriptPath, Notice: notice}
	select {
	case <-runCtx.Done():
		res.TimedOut = true
		if cmd.Process != nil {
			_ = syscall.Kill(-cmd.Process.Pid, syscall.SIGTERM)
			select {
			case <-waitCh:
			case <-time.After(defaults.ShutdownGrace):
				_ = syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL)
				<-waitCh
			}
		}
		res.Duration = time.Since(start)
		res.ExitCode = 124
		return res, fmt.Errorf("script timed out after %s", opts.Timeout)
	case err := <-waitCh:
		res.Duration = time.Since(start)
		if err != nil {
			var ee *exec.ExitError
			if errors.As(err, &ee) {
				res.ExitCode = ee.ExitCode()
				return res, nil
			}
			return res, err
		}
		return res, nil
	}
}
