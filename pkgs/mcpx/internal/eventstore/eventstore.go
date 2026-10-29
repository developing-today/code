// Package eventstore is the unified event store sketched in #1850: one
// table for every statistic, so a statistic becomes a query rather than
// a new code path. It lives BESIDE logstore, not instead of it — the
// log index answers "what happened", this store answers "how much,
// over time" (#1850's own lean, pinned by triage task 2740).
//
// First slice only: the event/kind/idset/idset_member schema, kind
// registration with JSON Schema validation of data, a writer API that
// rejects undeclared kinds, idset interning for identifier sets (the
// MCPX_TRACE_IDS pair shape), and one existing stats dimension (calls)
// re-expressed as a query over the store. Retention and the remaining
// dimensions are later slices.
package eventstore

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"sort"
	"strings"
	"sync"

	_ "modernc.org/sqlite"

	"github.com/dezren39/mcpx/internal/schemaval"
)

// Named rejection errors. A writer that invents a kind, or writes data
// its kind's schema forbids, gets one of these — never a silent insert.
var (
	ErrUndeclaredKind = errors.New("eventstore: undeclared kind")
	ErrInvalidData    = errors.New("eventstore: data fails the kind's JSON Schema")
	ErrKindConflict   = errors.New("eventstore: kind already registered with a different unit or schema")
)

// schemaDDL is #1850's sketched schema, verbatim in shape.
const schemaDDL = `
CREATE TABLE IF NOT EXISTS kind (
  id     INTEGER PRIMARY KEY,
  name   TEXT UNIQUE,
  unit   TEXT,
  schema TEXT
);
CREATE TABLE IF NOT EXISTS idset (
  id          INTEGER PRIMARY KEY,
  fingerprint TEXT UNIQUE
);
CREATE TABLE IF NOT EXISTS idset_member (
  idset_id INTEGER,
  key      TEXT,
  value    TEXT,
  PRIMARY KEY (idset_id, key, value)
);
CREATE TABLE IF NOT EXISTS event (
  id         INTEGER PRIMARY KEY,
  ts_unix_ms INTEGER NOT NULL,
  kind_id    INTEGER NOT NULL REFERENCES kind(id),
  idset_id   INTEGER REFERENCES idset(id),
  source     TEXT NOT NULL,
  value      REAL,
  data       TEXT
);
CREATE INDEX IF NOT EXISTS event_kind_ts ON event(kind_id, ts_unix_ms);
`

// Store is one event-store database.
type Store struct {
	db *sql.DB
	mu sync.Mutex
	// Compiled kind schemas, by kind name. Registration compiles once;
	// writes validate against the compiled form.
	schemas map[string]*schemaval.Schema
}

// openMu serialises Open; see the comment there.
var openMu sync.Mutex

// Open opens (creating if needed) the event store at path. The path
// ":memory:" gives a private in-memory store, as in tests.
func Open(path string) (*Store, error) {
	dsn := path
	if path != ":memory:" {
		// Same DSN discipline as logstore.Open: WAL, because a reader
		// must not block a writer, and busy_timeout first (pragmas
		// apply in order, and the WAL switch itself needs a lock) so
		// a concurrent writer from another Store instance over the
		// same file waits briefly instead of failing SQLITE_BUSY —
		// the cross-instance case RegisterKind and InternIDs are
		// written to survive (Codex review on PR #2449).
		dsn = path + "?_pragma=busy_timeout(5000)&_pragma=journal_mode(WAL)&_pragma=synchronous(NORMAL)"
	}
	// Schema creation upgrades a read lock to a write lock, which
	// SQLite refuses with SQLITE_BUSY without consulting busy_timeout
	// when another connection does the same; serialising opens in
	// this process removes that in-process race (as in logstore.Open).
	openMu.Lock()
	defer openMu.Unlock()
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, err
	}
	// SQLite serialises writers anyway; one connection keeps a
	// ":memory:" store a single database instead of one per pooled
	// connection.
	db.SetMaxOpenConns(1)
	if _, err := db.Exec(schemaDDL); err != nil {
		db.Close()
		return nil, fmt.Errorf("eventstore: creating schema: %w", err)
	}
	return &Store{db: db, schemas: map[string]*schemaval.Schema{}}, nil
}

// Close closes the underlying database.
func (s *Store) Close() error { return s.db.Close() }

// DB exposes the database for inspection and tests, as logstore does.
func (s *Store) DB() *sql.DB { return s.db }

// Kind is one declared event type.
type Kind struct {
	ID     int64
	Name   string
	Unit   string
	Schema string
}

// RegisterKind declares a kind. The schema is a JSON Schema document
// for the event's data, per #1850's lean (JSON Schema over a bespoke
// type system); an empty schema declares the kind unconstrained.
// Re-registering the same name with the same unit and schema is a
// no-op; with different ones it fails with ErrKindConflict, so two
// writers can never silently disagree about a kind's shape.
func (s *Store) RegisterKind(name, unit, schema string) (int64, error) {
	if name == "" {
		return 0, errors.New("eventstore: kind name is empty")
	}
	var compiled *schemaval.Schema
	if strings.TrimSpace(schema) != "" {
		c, err := schemaval.Compile(json.RawMessage(schema))
		if err != nil {
			return 0, fmt.Errorf("eventstore: compiling schema for kind %q: %w", name, err)
		}
		compiled = c
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	lookup := func() (int64, string, string, bool, error) {
		var id int64
		var haveUnit, haveSchema sql.NullString
		err := s.db.QueryRow(`SELECT id, unit, schema FROM kind WHERE name = ?`, name).Scan(&id, &haveUnit, &haveSchema)
		if errors.Is(err, sql.ErrNoRows) {
			return 0, "", "", false, nil
		}
		if err != nil {
			return 0, "", "", false, err
		}
		return id, haveUnit.String, haveSchema.String, true, nil
	}
	finish := func(id int64, haveUnit, haveSchema string) (int64, error) {
		if haveUnit != unit || haveSchema != schema {
			return 0, fmt.Errorf("%w: %q", ErrKindConflict, name)
		}
		if compiled != nil {
			s.schemas[name] = compiled
		}
		return id, nil
	}
	if id, haveUnit, haveSchema, found, err := lookup(); err != nil {
		return 0, err
	} else if found {
		return finish(id, haveUnit, haveSchema)
	}
	// Conflict-tolerant insert: another Store instance sharing this
	// database may register the same kind concurrently — s.mu only
	// serialises THIS instance. Losing that race is not an error: the
	// persisted definition is re-read and compared below, so identical
	// definitions succeed and differing ones get ErrKindConflict
	// either way (Codex review on PR #2449).
	if _, err := s.db.Exec(`INSERT INTO kind(name, unit, schema) VALUES (?, ?, ?) ON CONFLICT(name) DO NOTHING`, name, unit, schema); err != nil {
		return 0, err
	}
	id, haveUnit, haveSchema, found, err := lookup()
	if err != nil {
		return 0, err
	}
	if !found {
		return 0, fmt.Errorf("eventstore: kind %q missing after insert", name)
	}
	return finish(id, haveUnit, haveSchema)
}

// Kinds lists the declared kinds, by name.
func (s *Store) Kinds() ([]Kind, error) {
	rows, err := s.db.Query(`SELECT id, name, ifnull(unit,''), ifnull(schema,'') FROM kind ORDER BY name`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var out []Kind
	for rows.Next() {
		var k Kind
		if err := rows.Scan(&k.ID, &k.Name, &k.Unit, &k.Schema); err != nil {
			return nil, err
		}
		out = append(out, k)
	}
	return out, rows.Err()
}

// InternIDs interns one identifier set and returns its idset id. The
// pairs are the MCPX_TRACE_IDS shape — [key, value, more values...],
// where entries longer than a pair name several ids for one key
// (aliases for the same thing) — flattened here to (key, value)
// members. The same set, in any member order, interns to the same id:
// the set's fingerprint is a hash of its canonical member list.
func (s *Store) InternIDs(pairs [][]string) (int64, error) {
	memberSet := map[[2]string]struct{}{}
	for _, p := range pairs {
		if len(p) < 2 || p[0] == "" {
			continue
		}
		for _, v := range p[1:] {
			if v == "" {
				continue
			}
			memberSet[[2]string{p[0], v}] = struct{}{}
		}
	}
	members := make([][2]string, 0, len(memberSet))
	for m := range memberSet {
		members = append(members, m)
	}
	sort.Slice(members, func(i, j int) bool {
		if members[i][0] != members[j][0] {
			return members[i][0] < members[j][0]
		}
		return members[i][1] < members[j][1]
	})
	// The fingerprint hashes a structured (JSON) encoding of the
	// canonical member list. A separator-joined encoding would be
	// ambiguous: member strings may themselves contain the separator,
	// so distinct sets could collide onto one fingerprint and
	// silently share an idset (Codex review on PR #2449).
	canon, err := json.Marshal(members)
	if err != nil {
		return 0, err
	}
	sum := sha256.Sum256(canon)
	fingerprint := hex.EncodeToString(sum[:])

	s.mu.Lock()
	defer s.mu.Unlock()
	// The idset row and its members commit atomically: if a member
	// insert fails midway, nothing persists, so a retry re-runs the
	// member inserts instead of finding the fingerprint, skipping
	// them, and returning a permanently incomplete set (Codex review
	// on PR #2449).
	tx, err := s.db.BeginTx(context.Background(), nil)
	if err != nil {
		return 0, err
	}
	defer tx.Rollback()
	res, err := tx.Exec(`INSERT INTO idset(fingerprint) VALUES (?) ON CONFLICT(fingerprint) DO NOTHING`, fingerprint)
	if err != nil {
		return 0, err
	}
	var id int64
	if err := tx.QueryRow(`SELECT id FROM idset WHERE fingerprint = ?`, fingerprint).Scan(&id); err != nil {
		return 0, err
	}
	inserted, err := res.RowsAffected()
	if err != nil {
		return 0, err
	}
	if inserted == 1 {
		for _, m := range members {
			if _, err := tx.Exec(`INSERT INTO idset_member(idset_id, key, value) VALUES (?, ?, ?)`, id, m[0], m[1]); err != nil {
				return 0, err
			}
		}
	}
	if err := tx.Commit(); err != nil {
		return 0, err
	}
	return id, nil
}

// Event is one write. Data is the kind-specific payload, validated
// against the kind's JSON Schema; Value is the number, when there is
// one (a duration, a token count); IDs, when present, are interned and
// the event is stamped with their idset.
type Event struct {
	TsUnixMs int64
	Kind     string
	Source   string
	Value    *float64
	Data     json.RawMessage
	IDs      [][]string
}

// Write validates and records one event, returning its id. Writing an
// undeclared kind fails with ErrUndeclaredKind; data that fails the
// kind's schema fails with ErrInvalidData. Nothing is inserted in
// either case.
func (s *Store) Write(ev Event) (int64, error) {
	s.mu.Lock()
	var kindID int64
	var schemaText sql.NullString
	err := s.db.QueryRow(`SELECT id, schema FROM kind WHERE name = ?`, ev.Kind).Scan(&kindID, &schemaText)
	s.mu.Unlock()
	if errors.Is(err, sql.ErrNoRows) {
		return 0, fmt.Errorf("%w: %q", ErrUndeclaredKind, ev.Kind)
	}
	if err != nil {
		return 0, err
	}
	if schemaText.Valid && strings.TrimSpace(schemaText.String) != "" {
		s.mu.Lock()
		compiled, ok := s.schemas[ev.Kind]
		s.mu.Unlock()
		if !ok {
			// Registered by another Store over the same database (or
			// before a restart): compile on first use.
			c, err := schemaval.Compile(json.RawMessage(schemaText.String))
			if err != nil {
				return 0, fmt.Errorf("eventstore: compiling schema for kind %q: %w", ev.Kind, err)
			}
			compiled = c
			s.mu.Lock()
			s.schemas[ev.Kind] = compiled
			s.mu.Unlock()
		}
		data := ev.Data
		if len(data) == 0 {
			data = json.RawMessage("null")
		}
		if err := compiled.ValidateJSON(data); err != nil {
			return 0, fmt.Errorf("%w: kind %q: %v", ErrInvalidData, ev.Kind, err)
		}
	}
	var idsetID sql.NullInt64
	if len(ev.IDs) > 0 {
		id, err := s.InternIDs(ev.IDs)
		if err != nil {
			return 0, err
		}
		idsetID = sql.NullInt64{Int64: id, Valid: true}
	}
	var value sql.NullFloat64
	if ev.Value != nil {
		value = sql.NullFloat64{Float64: *ev.Value, Valid: true}
	}
	var dataText sql.NullString
	if len(ev.Data) > 0 {
		dataText = sql.NullString{String: string(ev.Data), Valid: true}
	}
	res, err := s.db.Exec(
		`INSERT INTO event(ts_unix_ms, kind_id, idset_id, source, value, data) VALUES (?, ?, ?, ?, ?, ?)`,
		ev.TsUnixMs, kindID, idsetID, ev.Source, value, dataText)
	if err != nil {
		return 0, err
	}
	return res.LastInsertId()
}
