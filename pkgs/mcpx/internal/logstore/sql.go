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
// rather than a value to assign. A bare PRAGMA (no value at all) is
// not automatically a question either — PRAGMA optimize and
// PRAGMA wal_checkpoint take no value and still change the database
// — so bare forms pass only when the name is in bareReadPragmas,
// the known reads.
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
// bareReadPragmas are the PRAGMAs whose bare form — no value, no
// argument list — is a read: the name alone asks SQLite about the
// database, a setting, or a check result, and changes nothing. Any
// other bare PRAGMA is refused, because several bare PRAGMAs are
// actions, not questions: PRAGMA optimize rewrites sqlite_stat1,
// PRAGMA wal_checkpoint forces WAL frames into the main database
// file, PRAGMA incremental_vacuum performs vacuum work (its bare
// form shrank the freelist in rollback-journal mode), and
// PRAGMA shrink_memory releases memory back to the OS. Those four
// are deliberately absent here; refusing unknown names by default
// keeps the next action PRAGMA out as well.
var bareReadPragmas = map[string]bool{
	"analysis_limit": true, "application_id": true, "auto_vacuum": true,
	"automatic_index": true, "busy_timeout": true, "cache_size": true,
	"cache_spill": true, "cell_size_check": true,
	"checkpoint_fullfsync": true, "collation_list": true,
	"compile_options": true, "data_version": true, "database_list": true,
	"defer_foreign_keys": true, "encoding": true,
	"foreign_key_check": true, "foreign_keys": true, "freelist_count": true,
	"full_column_names": true, "fullfsync": true, "function_list": true,
	"hard_heap_limit": true, "ignore_check_constraints": true,
	"integrity_check": true, "journal_mode": true,
	"journal_size_limit": true, "legacy_alter_table": true,
	"legacy_file_format": true, "locking_mode": true,
	"max_page_count": true, "max_recursive": true, "mmap_size": true,
	"module_list": true, "page_count": true, "page_size": true,
	"pragma_list": true, "query_only": true, "quick_check": true,
	"read_uncommitted":   true,
	"recursive_triggers": true, "reverse_unordered_selects": true,
	"schema_version": true, "secure_delete": true,
	"short_column_names": true, "soft_heap_limit": true,
	"synchronous": true, "table_list": true, "temp_store": true,
	"threads": true, "trusted_schema": true, "user_version": true,
	"wal_autocheckpoint": true, "wal_index": true,
	"writable_schema": true,
}

var functionFormReadPragmas = map[string]bool{
	"table_info": true, "table_xinfo": true, "table_list": true,
	"index_list": true, "index_info": true, "index_xinfo": true,
	"foreign_key_list": true, "foreign_key_check": true,
	"integrity_check": true, "quick_check": true,
}

// checkReadOnlyPragma refuses a PRAGMA that carries a value, and any
// bare PRAGMA whose name is not a known read. Bare reads (PRAGMA
// journal_mode, PRAGMA user_version) pass via bareReadPragmas; a value
// in either assignment form (= value or (value)) does not, except for
// the function-form reads above.
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
		return nil
	}
	// Bare form: no value and no argument list. The name alone must be
	// a known read — see bareReadPragmas. A schema qualifier
	// (main.wal_checkpoint) is stripped the same way as in the
	// function-form branch, so it cannot smuggle an action name past
	// the allowlist.
	fields := strings.Fields(rest)
	if len(fields) != 1 {
		return fmt.Errorf("PRAGMA %q is not a well-formed read PRAGMA", rest)
	}
	name := fields[0]
	if j := strings.LastIndex(name, "."); j >= 0 {
		name = name[j+1:]
	}
	if !bareReadPragmas[strings.ToLower(name)] {
		return fmt.Errorf("PRAGMA %q is not a known read PRAGMA; only the bare reads in the allowlist are allowed here", rest)
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
	defer func() { _ = rows.Close() }()
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
