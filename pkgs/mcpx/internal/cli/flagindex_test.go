package cli

import (
	"bytes"
	"fmt"
	"go/ast"
	"go/parser"
	"go/printer"
	"go/token"
	"os"
	"path/filepath"
	"reflect"
	"sort"
	"strconv"
	"strings"
	"testing"
)

// dynamicFlagFuncs are the functions whose flag set name is not a literal,
// with the flag-set names their declarations belong to. A function that
// builds a flag set from a computed name and is not listed here fails
// TestHandFlagIndexIsCurrent: a computed name is exactly where a new
// command's flags would slip past the index unnoticed.
var dynamicFlagFuncs = map[string][]string{
	// runScript serves both run and exec; the name is chosen by the
	// inline parameter, and both commands declare the same flags.
	"runScript": {"run", "exec"},
	// runOp declares its flags from the operation's own parameter table
	// (api.Op), which is already the single source for them, and names
	// the flag set after the operation group at run time. Nothing to
	// index: the one literal flag it adds, -o, is printed by its own
	// usage function from the same declaration.
	"runOp": {},
}

// scratchFlagReceivers are flag sets built inside one function to scan or
// copy another set's flags, keyed by file and receiver name. Their
// declarations are copies, not command flags; any other receiver that
// declares flags fails the test until it is classified here.
var scratchFlagReceivers = map[string]string{
	"settings.go:scratch": "copies the flags being scanned, name and all",
}

var flagDeclMethods = map[string]bool{
	"String": true, "Bool": true, "Int": true, "Int64": true,
	"Uint": true, "Uint64": true, "Float64": true, "Duration": true,
	"Var": true, "StringVar": true, "BoolVar": true, "IntVar": true,
	"Int64Var": true, "UintVar": true, "Uint64Var": true,
	"Float64Var": true, "DurationVar": true,
}

// constString evaluates a string expression made of literals and +
// concatenation, the only shapes a flag usage is written in. Anything else
// reports ok=false, and the caller fails: a usage that cannot be read
// statically cannot be indexed, and an unindexed flag is invisible.
func constString(e ast.Expr) (string, bool) {
	switch v := e.(type) {
	case *ast.BasicLit:
		if v.Kind != token.STRING {
			return "", false
		}
		s, err := strconv.Unquote(v.Value)
		return s, err == nil
	case *ast.BinaryExpr:
		if v.Op != token.ADD {
			return "", false
		}
		l, ok := constString(v.X)
		if !ok {
			return "", false
		}
		r, ok := constString(v.Y)
		return l + r, ok
	case *ast.ParenExpr:
		return constString(v.X)
	}
	return "", false
}

// usageString reads a flag's usage argument. Most usages are literals or
// concatenations of them; one (catalog --budget) formats a default in with
// fmt.Sprintf, and there the index records the argument's source in place
// of the verb -- "default `defaults.CatalogBudget`" -- because the value
// comes from an embedded builtin no static read can evaluate, and a
// number copied here would be a lie the moment the builtin changed.
func usageString(e ast.Expr) (string, bool) {
	if s, ok := constString(e); ok {
		return s, true
	}
	if s, ok := joinString(e); ok {
		return s, true
	}
	if sel, ok := e.(*ast.SelectorExpr); ok {
		if owner, ok := sel.X.(*ast.Ident); ok {
			if v, ok := resolveStringConst(owner.Name, sel.Sel.Name); ok {
				return v, true
			}
		}
		// A selector whose value is derived rather than literal
		// (defaults.ProtoMCPPath comes from the embedded builtin) is
		// recorded by source, like the Sprintf case above: the index
		// names where the value lives instead of copying one that
		// would silently go stale.
		var buf bytes.Buffer
		if err := printer.Fprint(&buf, token.NewFileSet(), e); err != nil {
			return "", false
		}
		return "`" + buf.String() + "`", true
	}
	if bin, ok := e.(*ast.BinaryExpr); ok && bin.Op == token.ADD {
		l, lok := usageString(bin.X)
		r, rok := usageString(bin.Y)
		if lok && rok {
			return l + r, true
		}
		return "", false
	}
	call, ok := e.(*ast.CallExpr)
	if !ok || len(call.Args) == 0 {
		return "", false
	}
	sel, ok := call.Fun.(*ast.SelectorExpr)
	if !ok || sel.Sel.Name != "Sprintf" {
		return "", false
	}
	pkg, ok := sel.X.(*ast.Ident)
	if !ok || pkg.Name != "fmt" {
		return "", false
	}
	format, ok := constString(call.Args[0])
	if !ok {
		return "", false
	}
	args := call.Args[1:]
	var b strings.Builder
	for i := 0; i < len(format); i++ {
		if format[i] == '%' && i+1 < len(format) && len(args) > 0 {
			j := i + 1
			for j < len(format) && !strings.ContainsRune("vdsqtbcxXUfeEgGsp", rune(format[j])) {
				j++
			}
			if j >= len(format) {
				return "", false
			}
			var buf bytes.Buffer
			if err := printer.Fprint(&buf, token.NewFileSet(), args[0]); err != nil {
				return "", false
			}
			b.WriteString("`" + buf.String() + "`")
			args = args[1:]
			i = j
			continue
		}
		b.WriteByte(format[i])
	}
	return b.String(), true
}

// resolveStringSlice finds a package-level []string literal in a sibling
// internal package: logstore.Dimensions is declared as one, and a usage
// that joins it into the flag's help can be read exactly.
func resolveStringSlice(pkgName, varName string) ([]string, bool) {
	files, err := filepath.Glob(filepath.Join("..", pkgName, "*.go"))
	if err != nil {
		return nil, false
	}
	for _, file := range files {
		if strings.HasSuffix(file, "_test.go") {
			continue
		}
		parsed, err := parser.ParseFile(token.NewFileSet(), file, nil, 0)
		if err != nil {
			continue
		}
		for _, decl := range parsed.Decls {
			gen, ok := decl.(*ast.GenDecl)
			if !ok {
				continue
			}
			for _, spec := range gen.Specs {
				vs, ok := spec.(*ast.ValueSpec)
				if !ok {
					continue
				}
				for i, name := range vs.Names {
					if name.Name != varName || i >= len(vs.Values) {
						continue
					}
					lit, ok := vs.Values[i].(*ast.CompositeLit)
					if !ok {
						return nil, false
					}
					var out []string
					for _, elt := range lit.Elts {
						v, ok := constString(elt)
						if !ok {
							return nil, false
						}
						out = append(out, v)
					}
					return out, true
				}
			}
		}
	}
	return nil, false
}

// resolveStringConst finds a package-level string literal -- a const or a
// var initialized with one -- in a sibling internal package:
// defaults.ProtoMCPPath is concatenated into serve --transport's usage.
func resolveStringConst(pkgName, varName string) (string, bool) {
	files, err := filepath.Glob(filepath.Join("..", pkgName, "*.go"))
	if err != nil {
		return "", false
	}
	for _, file := range files {
		if strings.HasSuffix(file, "_test.go") {
			continue
		}
		parsed, err := parser.ParseFile(token.NewFileSet(), file, nil, 0)
		if err != nil {
			continue
		}
		for _, decl := range parsed.Decls {
			gen, ok := decl.(*ast.GenDecl)
			if !ok {
				continue
			}
			for _, spec := range gen.Specs {
				vs, ok := spec.(*ast.ValueSpec)
				if !ok {
					continue
				}
				for i, name := range vs.Names {
					if name.Name != varName || i >= len(vs.Values) {
						continue
					}
					return constString(vs.Values[i])
				}
			}
		}
	}
	return "", false
}

// joinString evaluates strings.Join(pkg.Var, "sep") when pkg.Var is a
// literal string slice in a sibling package.
func joinString(e ast.Expr) (string, bool) {
	call, ok := e.(*ast.CallExpr)
	if !ok || len(call.Args) != 2 {
		return "", false
	}
	sel, ok := call.Fun.(*ast.SelectorExpr)
	if !ok || sel.Sel.Name != "Join" {
		return "", false
	}
	pkg, ok := sel.X.(*ast.Ident)
	if !ok || pkg.Name != "strings" {
		return "", false
	}
	arg, ok := call.Args[0].(*ast.SelectorExpr)
	if !ok {
		return "", false
	}
	owner, ok := arg.X.(*ast.Ident)
	if !ok {
		return "", false
	}
	sep, ok := constString(call.Args[1])
	if !ok {
		return "", false
	}
	vals, ok := resolveStringSlice(owner.Name, arg.Sel.Name)
	if !ok {
		return "", false
	}
	return strings.Join(vals, sep), true
}

// extractHandFlags reads this package's non-test sources and returns every
// hand-declared flag, keyed by flag-set name. A declaration is a call like
// fs.String("name", def, "usage") inside a function that created fs with
// newFlagSet. The extraction is deliberately strict -- an unclassifiable
// declaration is an error, not a skip -- because the failure this guards
// against is a flag quietly missing from help.
func extractHandFlags() (map[string][]HandFlag, error) {
	files, err := filepath.Glob("*.go")
	if err != nil {
		return nil, err
	}
	out := map[string][]HandFlag{}
	seen := map[string]map[string]bool{}
	add := func(scope string, f HandFlag) {
		if seen[scope] == nil {
			seen[scope] = map[string]bool{}
		}
		if seen[scope][f.Name] {
			return // two subcommands of one command may declare it alike
		}
		seen[scope][f.Name] = true
		out[scope] = append(out[scope], f)
	}
	for _, file := range files {
		if strings.HasSuffix(file, "_test.go") || file == "flagindex_generated.go" {
			continue
		}
		fset := token.NewFileSet()
		parsed, err := parser.ParseFile(fset, file, nil, 0)
		if err != nil {
			return nil, err
		}
		for _, decl := range parsed.Decls {
			fn, ok := decl.(*ast.FuncDecl)
			if !ok || fn.Body == nil || fn.Name.Name == "newFlagSet" {
				continue
			}
			// Which flag set each local variable is, for this function.
			setName := map[string][]string{}
			ast.Inspect(fn.Body, func(n ast.Node) bool {
				assign, ok := n.(*ast.AssignStmt)
				if !ok || len(assign.Lhs) != 1 || len(assign.Rhs) != 1 {
					return true
				}
				ident, ok := assign.Lhs[0].(*ast.Ident)
				if !ok {
					return true
				}
				call, ok := assign.Rhs[0].(*ast.CallExpr)
				if !ok {
					return true
				}
				if id, ok := call.Fun.(*ast.Ident); ok && id.Name == "newFlagSet" && len(call.Args) == 1 {
					if name, ok := constString(call.Args[0]); ok {
						setName[ident.Name] = []string{name}
					} else if scopes, ok := dynamicFlagFuncs[fn.Name.Name]; ok {
						setName[ident.Name] = scopes
					} else {
						setName[ident.Name] = nil // unclassified, flagged below
					}
				}
				if sel, ok := call.Fun.(*ast.SelectorExpr); ok && sel.Sel.Name == "NewFlagSet" {
					if len(call.Args) >= 1 {
						if name, ok := constString(call.Args[0]); ok {
							setName[ident.Name] = []string{"\x00scratch:" + name}
						}
					}
				}
				return true
			})
			var fnErr error
			ast.Inspect(fn.Body, func(n ast.Node) bool {
				if fnErr != nil {
					return false
				}
				call, ok := n.(*ast.CallExpr)
				if !ok {
					return true
				}
				sel, ok := call.Fun.(*ast.SelectorExpr)
				if !ok || !flagDeclMethods[sel.Sel.Name] {
					return true
				}
				recv, ok := sel.X.(*ast.Ident)
				if !ok {
					return true
				}
				if len(call.Args) < 3 {
					// A one-argument String/Bool/Int/Duration call is a
					// settings read (set.Duration("http.requestTimeout")),
					// not a declaration: a declaration always carries a
					// name, a default and a usage.
					return true
				}
				scopes, known := setName[recv.Name]
				if !known {
					if _, ok := scratchFlagReceivers[file+":"+recv.Name]; ok {
						return true
					}
					fnErr = fmt.Errorf("%s: %s declares flag %s on %q, which this function did not create with newFlagSet; classify it in flagindex_test.go",
						file, fn.Name.Name, sel.Sel.Name, recv.Name)
					return false
				}
				if scopes == nil {
					fnErr = fmt.Errorf("%s: %s builds a flag set from a computed name; add it to dynamicFlagFuncs in flagindex_test.go",
						file, fn.Name.Name)
					return false
				}
				if len(scopes) == 0 {
					return true // classified dynamic set: flags come from another source
				}
				scratchOnly := true
				for _, scope := range scopes {
					if !strings.HasPrefix(scope, "\x00scratch:") {
						scratchOnly = false
					}
				}
				if scratchOnly {
					return true // a copy of another set's flags, names and all
				}
				// Var and the *Var family take the value first, so the
				// name is the second argument; the usage is last either way.
				nameArg := 0
				if strings.HasSuffix(sel.Sel.Name, "Var") {
					nameArg = 1
				}
				if len(call.Args) <= nameArg {
					return true
				}
				name, ok := constString(call.Args[nameArg])
				if !ok {
					fnErr = fmt.Errorf("%s: %s declares a flag with a computed name; flag names must be literals to be indexed",
						file, fn.Name.Name)
					return false
				}
				usage, ok := usageString(call.Args[len(call.Args)-1])
				if !ok {
					fnErr = fmt.Errorf("%s: flag --%s has a usage that is not a string literal or concatenation; it cannot be indexed",
						file, name)
					return false
				}
				for _, scope := range scopes {
					if strings.HasPrefix(scope, "\x00scratch:") {
						continue
					}
					add(scope, HandFlag{Name: name, Usage: usage})
				}
				return true
			})
			if fnErr != nil {
				return nil, fnErr
			}
		}
	}
	return out, nil
}

// renderHandFlagIndex spells the extracted index the way
// flagindex_generated.go carries it, so the comparison is over the exact
// bytes a reader of the generated file sees.
func renderHandFlagIndex(index map[string][]HandFlag) string {
	scopes := make([]string, 0, len(index))
	for scope := range index {
		scopes = append(scopes, scope)
	}
	sort.Strings(scopes)
	var b strings.Builder
	b.WriteString("var handFlagIndex = map[string][]HandFlag{\n")
	for _, scope := range scopes {
		fmt.Fprintf(&b, "\t%q: {\n", scope)
		for _, f := range index[scope] {
			fmt.Fprintf(&b, "\t\t{Name: %q, Usage: %q},\n", f.Name, f.Usage)
		}
		b.WriteString("\t},\n")
	}
	b.WriteString("}\n")
	return b.String()
}

// TestHandFlagIndexIsCurrent is the drift guard for #2516 item 8: the
// generated index must equal what the declarations in this package
// actually say. Declare a flag by hand and this test fails until the
// index is regenerated, at which point help, the man page and completion
// all show it -- the flag has one declaration and every surface reads it.
func TestHandFlagIndexIsCurrent(t *testing.T) {
	want, err := extractHandFlags()
	if err != nil {
		t.Fatal(err)
	}
	if *update {
		raw, err := os.ReadFile("flagindex_generated.go")
		if err != nil {
			t.Fatal(err)
		}
		head := string(raw)
		i := strings.Index(head, "var handFlagIndex")
		if i < 0 {
			t.Fatal("flagindex_generated.go has lost its index declaration")
		}
		if err := os.WriteFile("flagindex_generated.go",
			[]byte(head[:i]+renderHandFlagIndex(want)), 0o644); err != nil {
			t.Fatal(err)
		}
		return
	}
	if !reflect.DeepEqual(normalizeIndex(want), normalizeIndex(handFlagIndex)) {
		t.Errorf("flagindex_generated.go is stale; run\n  go test ./internal/cli/ -run TestHandFlagIndexIsCurrent -update")
	}
}

func normalizeIndex(in map[string][]HandFlag) map[string][]HandFlag {
	out := make(map[string][]HandFlag, len(in))
	for scope, flags := range in {
		cp := append([]HandFlag(nil), flags...)
		out[scope] = cp
	}
	return out
}

// TestHandFlagsReachEverySurface is the presence half of the guard: every
// indexed flag must actually appear in its command's help, in the man
// page, and in all three completion scripts. The index being current
// proves nothing if a surface stops reading it.
func TestHandFlagsReachEverySurface(t *testing.T) {
	man := ManPage("0.0.0-test")
	scripts := map[string]string{}
	for _, shell := range []string{"bash", "zsh", "fish"} {
		out, err := Completion(shell, "0.0.0-test")
		if err != nil {
			t.Fatal(err)
		}
		scripts[shell] = out
	}
	helps := map[string]string{}
	for _, c := range Commands() {
		text, err := (&App{}).helpText(c.Name)
		if err != nil {
			t.Fatal(err)
		}
		helps[c.Name] = text
	}
	for scope, flags := range handFlagIndex {
		cmd := strings.Fields(scope)[0]
		help, ok := helps[cmd]
		if !ok {
			t.Errorf("flag set %q names no command in Commands(); its flags have no help to reach", scope)
			continue
		}
		for _, f := range flags {
			if !strings.Contains(help, "--"+f.Name) {
				t.Errorf("help for %s does not mention --%s (declared for %s)", cmd, f.Name, scope)
			}
			// The man page roff-escapes hyphens in flag names, the
			// same rendering its SETTINGS section has always used.
			if needle := "--" + strings.ReplaceAll(f.Name, "-", `\-`); !strings.Contains(man, needle) {
				t.Errorf("man page does not mention --%s (declared for %s)", f.Name, scope)
			}
			for shell, script := range scripts {
				if !strings.Contains(script, f.Name) {
					t.Errorf("%s completion does not mention %s (declared for %s)", shell, f.Name, scope)
				}
			}
		}
	}
}
