package auth

import (
	"go/ast"
	"go/parser"
	"go/token"
	"path/filepath"
	"runtime"
	"strconv"
	"testing"
)

// The functions below receive credential material: ed25519 private keys
// (InsertPrivateKey) and the password-derived hash/text values that stand in
// for SSH passwords (InsertHashPublicKey, GetPublicKeyFromHash,
// InsertTextPublicKey, GetPublicKeyFromText, GetPublicKey). None of that
// material, nor any value derived from it, may reach a log record: anyone
// with journal or log-collection access could otherwise recover credentials
// or replayable equivalents. See issue #6181.
func TestAuthenticationLogsDoNotReferenceCredentialMaterial(t *testing.T) {
	_, testFile, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("could not locate test source")
	}
	source := filepath.Join(filepath.Dir(testFile), "auth.go")
	file, err := parser.ParseFile(token.NewFileSet(), source, nil, 0)
	if err != nil {
		t.Fatal(err)
	}

	protectedFunctions := map[string]bool{
		"InsertPrivateKey":     true,
		"InsertHashPublicKey":  true,
		"GetPublicKeyFromHash": true,
		"InsertTextPublicKey":  true,
		"GetPublicKeyFromText": true,
		"GetPublicKey":         true,
	}
	forbiddenIdentifiers := map[string]bool{
		"pk":               true,
		"privateKeyString": true,
		"hash":             true,
		"text":             true,
		"password":         true,
	}
	forbiddenFields := map[string]bool{
		"privateKeyString": true,
		"private-key":      true,
		"private_key":      true,
		"hash":             true,
		"text":             true,
		"password":         true,
	}

	for _, declaration := range file.Decls {
		function, ok := declaration.(*ast.FuncDecl)
		if !ok || !protectedFunctions[function.Name.Name] {
			continue
		}
		ast.Inspect(function.Body, func(node ast.Node) bool {
			call, ok := node.(*ast.CallExpr)
			if !ok || !isLoggerCall(call.Fun) {
				return true
			}
			ast.Inspect(call, func(argument ast.Node) bool {
				switch value := argument.(type) {
				case *ast.Ident:
					if forbiddenIdentifiers[value.Name] {
						t.Errorf("%s logs credential-bearing identifier %q", function.Name.Name, value.Name)
					}
				case *ast.BasicLit:
					if value.Kind == token.STRING {
						field, err := strconv.Unquote(value.Value)
						if err == nil && forbiddenFields[field] {
							t.Errorf("%s logs credential-bearing field %q", function.Name.Name, field)
						}
					}
				}
				return true
			})
			return true
		})
	}
}

func isLoggerCall(expression ast.Expr) bool {
	selector, ok := expression.(*ast.SelectorExpr)
	if !ok {
		return false
	}
	logger, ok := selector.X.(*ast.Ident)
	if !ok || logger.Name != "log" {
		return false
	}
	switch selector.Sel.Name {
	case "Debug", "Info", "Warn", "Error", "Fatal", "Print":
		return true
	default:
		return false
	}
}
