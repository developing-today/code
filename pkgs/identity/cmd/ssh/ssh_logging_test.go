package ssh

import (
	"go/ast"
	"go/parser"
	"go/token"
	"path/filepath"
	"runtime"
	"strconv"
	"testing"
)

func TestAuthenticationLogsDoNotReferenceCredentialMaterial(t *testing.T) {
	_, testFile, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("could not locate test source")
	}
	source := filepath.Join(filepath.Dir(testFile), "ssh.go")
	file, err := parser.ParseFile(token.NewFileSet(), source, nil, 0)
	if err != nil {
		t.Fatal(err)
	}

	protectedFunctions := map[string]bool{
		"StartSsh":             true,
		"Connect":              true,
		"TeaHandlerWithStream": true,
	}
	forbiddenIdentifiers := map[string]bool{
		"password":                  true,
		"passwordHash":              true,
		"passwordSha256":            true,
		"passwordSha256Str":         true,
		"interactive":               true,
		"interactiveStr":            true,
		"ed25519PrivateKey":         true,
		"pkMelted":                  true,
		"meltedPrivateKeySeed":      true,
		"permissionsExtensionsJson": true,
	}
	forbiddenFields := map[string]bool{
		"password":              true,
		"passwordHash":          true,
		"passwordSha256":        true,
		"response":              true,
		"interactiveStr":        true,
		"permissionsExtensions": true,
		"private-key":           true,
		"private-key-seed":      true,
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
				case *ast.SelectorExpr:
					receiver, ok := value.X.(*ast.Ident)
					if ok && receiver.Name == "connection" && (value.Sel.Name == "ToData" || value.Sel.Name == "String") {
						t.Errorf("%s logs serialized authentication data via %s", function.Name.Name, value.Sel.Name)
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
