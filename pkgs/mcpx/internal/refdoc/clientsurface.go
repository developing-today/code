package refdoc

import (
	"bytes"
	"go/ast"
	"go/parser"
	"go/printer"
	"go/token"
	"sort"
	"strings"
)

// ClientSurface renders the public surface of a Go package from its
// source: exported type declarations, exported functions, and exported
// methods whose receiver type is itself exported. Each row carries the
// first paragraph of the declaration's doc comment, so the reference
// page's explanation is written next to the code it describes instead
// of in the page (#13612, slice of #13264).
//
// The input is the package's non-test Go source keyed by file name; a
// map rather than a directory so the drift tests can feed disposable
// fixtures through the same rendering the committed region uses. Files
// are read in sorted order and rows are sorted, so the output is
// deterministic — the region test compares it byte for byte.
//
// What is deliberately absent is the part that must not drift the
// reference: an implementation body, an unexported declaration, a
// struct's fields, and the value expression of a package-level
// variable. A signature is a declaration's name, type parameters,
// receiver, parameter types and result types, nothing else.
func ClientSurface(files map[string][]byte) (string, error) {
	names := make([]string, 0, len(files))
	for name := range files {
		names = append(names, name)
	}
	sort.Strings(names)

	type row struct{ sig, doc string }
	var rows []row

	for _, name := range names {
		fset := token.NewFileSet()
		file, err := parser.ParseFile(fset, name, files[name], parser.ParseComments)
		if err != nil {
			return "", err
		}
		render := func(n ast.Node) (string, error) {
			var b bytes.Buffer
			if err := printer.Fprint(&b, fset, n); err != nil {
				return "", err
			}
			// A type expression the printer wrapped over several lines
			// still has to fit one table cell.
			return strings.Join(strings.Fields(b.String()), " "), nil
		}

		for _, d := range file.Decls {
			switch d := d.(type) {
			case *ast.GenDecl:
				if d.Tok != token.TYPE {
					continue
				}
				for _, spec := range d.Specs {
					ts, ok := spec.(*ast.TypeSpec)
					if !ok || !ts.Name.IsExported() {
						continue
					}
					head := "type " + ts.Name.Name
					if ts.TypeParams != nil {
						tp, err := render(ts.TypeParams)
						if err != nil {
							return "", err
						}
						head += tp
					}
					switch ts.Type.(type) {
					case *ast.StructType:
						head += " struct"
					case *ast.InterfaceType:
						head += " interface"
					default:
						t, err := render(ts.Type)
						if err != nil {
							return "", err
						}
						head += " " + t
					}
					rows = append(rows, row{head, docSummary(ts.Doc, d.Doc)})
				}
			case *ast.FuncDecl:
				if !d.Name.IsExported() || d.Type == nil {
					continue
				}
				typ, err := render(d.Type)
				if err != nil {
					return "", err
				}
				body := strings.TrimPrefix(typ, "func")
				if d.Recv == nil {
					rows = append(rows, row{d.Name.Name + body, docSummary(d.Doc)})
					continue
				}
				recv, ok := exportedReceiver(d.Recv)
				if !ok {
					continue
				}
				rt, err := render(recv)
				if err != nil {
					return "", err
				}
				rows = append(rows, row{"(" + rt + ")." + d.Name.Name + body, docSummary(d.Doc)})
			}
		}
	}

	// Types first, then functions, then methods grouped by receiver:
	// the order a reader looks things up in. The sort is stable so two
	// declarations with one rendered signature keep source order.
	kind := func(sig string) int {
		switch {
		case strings.HasPrefix(sig, "type "):
			return 0
		case strings.HasPrefix(sig, "("):
			return 2
		default:
			return 1
		}
	}
	sort.SliceStable(rows, func(i, j int) bool {
		ki, kj := kind(rows[i].sig), kind(rows[j].sig)
		if ki != kj {
			return ki < kj
		}
		return rows[i].sig < rows[j].sig
	})

	body := make([][]string, 0, len(rows))
	for _, r := range rows {
		body = append(body, []string{"`" + r.sig + "`", r.doc})
	}
	return Table([]string{"signature", "what it is"}, body), nil
}

// exportedReceiver unwraps a method receiver to the type expression to
// render, reporting false for an unexported receiver type: a method on
// an unexported type is not part of the package's public surface.
func exportedReceiver(recv *ast.FieldList) (ast.Expr, bool) {
	if recv == nil || len(recv.List) == 0 {
		return nil, false
	}
	var base *ast.Ident
	switch t := recv.List[0].Type.(type) {
	case *ast.StarExpr:
		switch x := t.X.(type) {
		case *ast.Ident:
			base = x
		case *ast.IndexExpr:
			base, _ = x.X.(*ast.Ident)
		case *ast.IndexListExpr:
			base, _ = x.X.(*ast.Ident)
		}
	case *ast.Ident:
		base = t
	case *ast.IndexExpr:
		base, _ = t.X.(*ast.Ident)
	case *ast.IndexListExpr:
		base, _ = t.X.(*ast.Ident)
	}
	if base == nil || !base.IsExported() {
		return nil, false
	}
	return recv.List[0].Type, true
}

// docSummary is the first paragraph of a doc comment, whitespace
// collapsed onto one line, which is what fits a table cell. It accepts
// a doc comment either from the spec or from the enclosing declaration:
// a single-spec `type X ...` carries its comment on the GenDecl.
func docSummary(cgs ...*ast.CommentGroup) string {
	for _, cg := range cgs {
		if cg == nil {
			continue
		}
		text := cg.Text()
		if i := strings.Index(text, "\n\n"); i >= 0 {
			text = text[:i]
		}
		text = strings.Join(strings.Fields(text), " ")
		if text != "" {
			return text
		}
	}
	return ""
}
