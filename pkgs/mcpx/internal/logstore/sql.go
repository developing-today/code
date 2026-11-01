package logstore

import (
	"fmt"
	"strings"
)

// readOnlyVerbs are the statements `mcpx log sql` will run.
//
// This is a guard rail, not a sandbox. The index can be deleted and rebuilt
// from the JSONL at any time, so nothing here is protecting data; it is
// protecting someone who typed DELETE meaning SELECT from a confusing
// afternoon. PRAGMA is allowed because that is how you ask SQLite about
// itself. A PRAGMA that carries a value is not a question, it is an
// assignment, so CheckReadOnly refuses value-carrying PRAGMAs in both
// forms (name = value and name(value)); the only parenthesised forms
// allowed are the established function-form reads in
// functionFormReadPragmas, whose argument names the object to inspect
// rather than a value to assign.
var readOnlyVerbs = map[string]bool{
	"select": true, "explain": true, "pragma": true, "with": true, "values": true,
}

// CheckReadOnly rejects a statement that would change anything.
func CheckReadOnly(query string) error {
	stripped := stripSQLComments(query)
	trimmed := strings.TrimSpace(stripped)
	if trimmed == "" {
		return fmt.Errorf("empty query")
	}
	if strings.Contains(strings.TrimSuffix(strings.TrimSpace(trimmed), ";"), ";") {
		return fmt.Errorf("one statement at a time")
	}
	verb := strings.ToLower(strings.FieldsFunc(trimmed, func(r rune) bool {
		return r == ' ' || r == '\t' || r == '\n' || r == '(' || r == ';'
	})[0])
	if !readOnlyVerbs[verb] {
		return fmt.Errorf("%q is not a read-only statement; use SELECT, WITH, VALUES, EXPLAIN or PRAGMA", verb)
	}
	if verb == "pragma" {
		if err := checkReadOnlyPragma(trimmed); err != nil {
			return err
		}
	}
	// A CTE can hide a writer: WITH x AS (...) DELETE FROM records is valid
	// SQLite, and checking only the first word would wave it through.
	if verb == "with" {
		lower := strings.ToLower(trimmed)
		for _, bad := range []string{"delete", "insert", "update", "drop", "alter", "create", "replace"} {
			if containsWord(lower, bad) {
				return fmt.Errorf("%q appears in the query; only reads are allowed here", bad)
			}
		}
	}
	return nil
}

// functionFormReadPragmas are the PRAGMAs whose parenthesised form is a
// read: the argument names a table, index, or check to inspect, it is not
// a value being assigned to the pragma. Any other parenthesised PRAGMA
// (e.g. user_version(42), journal_mode(DELETE)) is an assignment in call
// form and is refused.
var functionFormReadPragmas = map[string]bool{
	"table_info": true, "table_xinfo": true,
	"index_list": true, "index_info": true, "index_xinfo": true,
	"foreign_key_list": true, "foreign_key_check": true,
	"integrity_check": true, "quick_check": true,
}

// checkReadOnlyPragma refuses a PRAGMA that carries a value. Bare PRAGMAs
// (PRAGMA journal_mode, PRAGMA user_version) are reads and pass; a value
// in either assignment form (= value or (value)) does not, except for the
// function-form reads above.
func checkReadOnlyPragma(trimmed string) error {
	rest := strings.TrimSpace(trimmed[len("pragma"):])
	rest = strings.TrimSpace(strings.TrimSuffix(rest, ";"))
	if rest == "" {
		return fmt.Errorf("PRAGMA with no name is not a read-only statement")
	}
	if strings.Contains(rest, "=") {
		return fmt.Errorf("PRAGMA %q carries a value; only read PRAGMAs with no assigned value are allowed here", rest)
	}
	if i := strings.Index(rest, "("); i >= 0 {
		if !strings.HasSuffix(rest, ")") {
			return fmt.Errorf("PRAGMA %q is not a well-formed read PRAGMA", rest)
		}
		name := rest[:i]
		if j := strings.LastIndex(name, "."); j >= 0 {
			name = name[j+1:]
		}
		if !functionFormReadPragmas[strings.ToLower(strings.TrimSpace(name))] {
			return fmt.Errorf("PRAGMA %q carries a value; only read PRAGMAs with no assigned value are allowed here", rest)
		}
	}
	return nil
}

func containsWord(s, word string) bool {
	for i := 0; ; {
		j := strings.Index(s[i:], word)
		if j < 0 {
			return false
		}
		j += i
		before := j == 0 || !isWordByte(s[j-1])
		after := j+len(word) >= len(s) || !isWordByte(s[j+len(word)])
		if before && after {
			return true
		}
		i = j + len(word)
	}
}

func isWordByte(b byte) bool {
	return b == '_' || (b >= 'a' && b <= 'z') || (b >= 'A' && b <= 'Z') || (b >= '0' && b <= '9')
}

// stripSQLComments removes -- and /* */ comments so a verb cannot be hidden
// behind one.
func stripSQLComments(q string) string {
	var b strings.Builder
	for i := 0; i < len(q); {
		switch {
		case strings.HasPrefix(q[i:], "--"):
			n := strings.IndexByte(q[i:], '\n')
			if n < 0 {
				return b.String()
			}
			i += n
		case strings.HasPrefix(q[i:], "/*"):
			n := strings.Index(q[i+2:], "*/")
			if n < 0 {
				return b.String()
			}
			i += n + 4
		default:
			b.WriteByte(q[i])
			i++
		}
	}
	return b.String()
}

// Table is the result of a raw query, already stringified for printing.
type Table struct {
	Columns []string   `json:"columns"`
	Rows    [][]string `json:"rows"`
}

// RunSQL executes a checked read-only statement.
func (s *Store) RunSQL(query string) (*Table, error) {
	if err := CheckReadOnly(query); err != nil {
		return nil, err
	}
	rows, err := s.db.Query(query)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	cols, err := rows.Columns()
	if err != nil {
		return nil, err
	}
	out := &Table{Columns: cols}
	for rows.Next() {
		cells := make([]any, len(cols))
		for i := range cells {
			cells[i] = new(any)
		}
		if err := rows.Scan(cells...); err != nil {
			return nil, err
		}
		row := make([]string, len(cols))
		for i, c := range cells {
			row[i] = cellString(*(c.(*any)))
		}
		out.Rows = append(out.Rows, row)
	}
	return out, rows.Err()
}

func cellString(v any) string {
	switch x := v.(type) {
	case nil:
		return ""
	case []byte:
		return string(x)
	case string:
		return x
	}
	return fmt.Sprint(v)
}
