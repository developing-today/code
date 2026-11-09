// Package tasks holds background requests and their results.
//
// A client opts in by asking for a task instead of a result. Instead of
// waiting it gets a handle back immediately, and polls for status or blocks
// for the result. That is the right shape for a genuinely slow call -- a
// build, a crawl, a browser session -- where holding a request open for
// minutes invites every intermediary to time it out.
//
// The store lives here rather than beside either caller because both the MCP
// server and the daemon's /v1 offer the same thing. Two stores would be two
// sets of expiry bugs, and a task started over one surface would be
// invisible from the other.
package tasks

import (
	"context"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"sort"
	"sync"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
	"github.com/developing-today/code/pkgs/mcpx/internal/logging"
	_ "modernc.org/sqlite"
)

// Task is one background request.
type Task struct {
	TaskID        string    `json:"taskId"`
	Status        string    `json:"status"`
	StatusMessage string    `json:"statusMessage,omitempty"`
	CreatedAt     time.Time `json:"createdAt"`
	LastUpdatedAt time.Time `json:"lastUpdatedAt"`
	// ExpiresAt is the terminal-result retention deadline. Nonterminal tasks
	// have no retention deadline and serialize it as null.
	ExpiresAt *time.Time `json:"expiresAt"`
	// LastTouched is creation time or the latest public item/result read.
	LastTouched *time.Time `json:"lastTouched"`
	// TTLRemainingMs is refreshed for each representation and is zero when
	// no terminal-result retention deadline exists.
	TTLRemainingMs int64 `json:"ttlRemainingMs"`
	// TTL is how long a finished task's result is kept, in milliseconds,
	// measured from when the task reaches a terminal state — never from
	// creation: a task still working when its TTL passes keeps working,
	// and its retention clock starts when it finishes. Null in the MCP
	// specification means unbounded; mcpx never offers unbounded, because
	// a result nobody collects is memory nobody frees.
	TTL          int64 `json:"ttl"`
	PollInterval int64 `json:"pollInterval,omitempty"`
	// InputRequests are the questions an input_required task is waiting
	// on, keyed as the client answers them. The tasks extension carries them
	// on tasks/get; 2025-11-25 has no field for them, so they are not part
	// of this shape. Replaced whole, never mutated, so a snapshot may share
	// it.
	InputRequests map[string]any `json:"-"`

	// requestor is the key the task counts against for the store's
	// per-requestor limits (protoTasks.maxPerRequestor). Never on the wire.
	requestor Requestor

	result any
	fault  *Fault
	cancel context.CancelFunc
	done   chan struct{}
	// doneClosed, guarded by Store.mu, makes closing done once-only:
	// grace expiry closes it without the work's return, and the worker
	// goroutine's close after finish must not close it a second time.
	doneClosed bool
	timer      *time.Timer
	// cancelTimer bounds the in-memory cooperative-cancellation window. The
	// cancelling mark and timer are deliberately not persisted or exposed.
	cancelTimer *time.Timer
	cancelling  bool
	OnCancel    func(ctx context.Context, reason string) `json:"-"`
}

func deadline(terminalAt, lastTouched time.Time, ttl, maxAge time.Duration) time.Time {
	expires := lastTouched.Add(ttl)
	if maxAge > 0 {
		if ceiling := terminalAt.Add(maxAge); ceiling.Before(expires) {
			expires = ceiling
		}
	}
	return expires
}

func (t *Task) refreshTTLRemaining(now time.Time) {
	if t.ExpiresAt == nil {
		t.TTLRemainingMs = 0
		return
	}
	remaining := t.ExpiresAt.Sub(now).Milliseconds()
	if remaining < 0 {
		remaining = 0
	}
	t.TTLRemainingMs = remaining
}

func snapshot(t *Task, now time.Time) Task {
	out := *t
	if t.ExpiresAt != nil {
		expires := *t.ExpiresAt
		out.ExpiresAt = &expires
	}
	if t.LastTouched != nil {
		lastTouched := *t.LastTouched
		out.LastTouched = &lastTouched
	}
	out.refreshTTLRemaining(now)
	return out
}

// Statuses, from the specification.
const (
	Working       = "working"
	InputRequired = "input_required"
	Completed     = "completed"
	Failed        = "failed"
	Cancelled     = "cancelled"
)

// Terminal reports whether a status is final.
func Terminal(status string) bool {
	return status == Completed || status == Failed || status == Cancelled
}

// Fault is why a task failed, carrying the JSON-RPC code so that an MCP
// client gets the error its transport defines rather than a flattened
// string.
type Fault struct {
	Code    int    `json:"code"`
	Message string `json:"message"`
	Data    any    `json:"data,omitempty"`
}

func (f *Fault) Error() string { return f.Message }

// Store holds running and recently finished tasks.
type Store struct {
	mu     sync.Mutex
	tasks  map[string]*Task
	db     *sql.DB
	ttl    time.Duration
	maxAge time.Duration
	// Audit receives task lifecycle records. Set it before the store is
	// shared; records contain identifiers and metadata, never task contents.
	Audit func(logging.Record)
	// PollInterval is the pollInterval each new task carries. Zero means
	// the built-in default. Set before the store is shared.
	PollInterval time.Duration

	// MaxTTL clamps any requested TTL inside Start. Zero means
	// DefaultMaxTTL. Wired from protoTasks.maxTTL by the seams.
	MaxTTL time.Duration
	// MaxPerRequestor caps one requestor's non-terminal tasks. Zero means
	// DefaultMaxPerRequestor. Wired from protoTasks.maxPerRequestor.
	MaxPerRequestor int
	// PerMinute is the tasks/* admission window per requestor. Zero means
	// DefaultPerMinute; a negative value disables the window. Wired from
	// protoTasks.perMinute (whose 0 means disabled, translated by the
	// wiring seam, not here).
	PerMinute int
	// CancelGrace is how long cooperative work may finish after cancellation
	// is requested. Zero means DefaultCancelGrace.
	CancelGrace time.Duration

	win      *Window
	winLimit int

	// watchers hear the id of every task whose state changed, after the
	// change and outside the lock, so a watcher may read the store.
	watchers map[int]func(id string)
	nextW    int

	// OnCancelHook is called when a task is cancelled to perform cooperative
	// upstream cancellation (e.g. notifications/cancelled or process interrupt)
	OnCancelHook func(id string, reason string)
}

// Configure updates the default result TTL and its terminal-time ceiling.
// Nonpositive values leave the built-in/current configuration unchanged.
func (s *Store) Configure(ttl, maxAge time.Duration) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if ttl > 0 {
		s.ttl = ttl
	}
	if maxAge > 0 {
		s.maxAge = maxAge
	}
}

// Watch calls fn with a task's id each time its status, input requests or
// status message change -- which is what notifications/tasks reports. The
// call comes after the change is visible to Get, and for a finished task
// after Result has stopped blocking. stop ends the watch.
func (s *Store) Watch(fn func(id string)) (stop func()) {
	s.mu.Lock()
	if s.watchers == nil {
		s.watchers = map[int]func(string){}
	}
	n := s.nextW
	s.nextW++
	s.watchers[n] = fn
	s.mu.Unlock()
	return func() {
		s.mu.Lock()
		delete(s.watchers, n)
		s.mu.Unlock()
	}
}

// changed tells the watchers. Called without the lock held.
func (s *Store) changed(id string) {
	s.mu.Lock()
	fns := make([]func(string), 0, len(s.watchers))
	for _, fn := range s.watchers {
		fns = append(fns, fn)
	}
	s.mu.Unlock()
	for _, fn := range fns {
		fn(id)
	}
}

func (s *Store) audit(level slog.Level, event string, attrs map[string]any) {
	if s.Audit == nil {
		return
	}
	attrs[logging.KeyEvent] = event
	s.Audit(logging.Record{Time: time.Now(), Level: level, Msg: event, Attrs: attrs})
}

func requestorAuditAttrs(requestor Requestor) map[string]any {
	key := requestor.Identity
	if key == "" {
		key = "unbound"
	}
	return map[string]any{"requestor_key": key, "requestor_role": requestor.Role}
}

func (s *Store) logTerminal(t *Task) {
	s.audit(slog.LevelInfo, "task.terminal", map[string]any{
		"task_id":     t.TaskID,
		"status":      t.Status,
		"duration_ms": time.Since(t.CreatedAt).Milliseconds(),
	})
}

// RecordRetrieval logs the metadata for a task retrieval at debug level.
// The surface and method are supplied by the handler; task data is not read.
func (s *Store) RecordRetrieval(surface, method, id string) {
	s.audit(slog.LevelDebug, "task.retrieval", map[string]any{
		"surface": surface,
		"method":  method,
		"task_id": id,
	})
}

type idKey struct{}

// IDFrom is the id of the task whose body ctx belongs to, or "". It is how a
// call running as a task finds the task it should report its status into.
func IDFrom(ctx context.Context) string {
	id, _ := ctx.Value(idKey{}).(string)
	return id
}

// New builds an empty store.
func New() *Store {
	return &Store{tasks: map[string]*Task{}, ttl: defaults.TaskTTL, maxAge: defaults.TaskMaxAge}
}

// Open opens or creates a persistent SQLite-backed task store.
func Open(dbPath string) (*Store, error) {
	return OpenWithRetention(dbPath, 0, 0)
}

// OpenWithRetention is Open with the deployment's retention settings
// applied before the store loads and migrates existing rows. That
// ordering matters: loadTasksFromDB derives deadlines for legacy rows
// (expires_ms unset) from the store's ttl/maxAge and deletes rows past
// them, so configuring retention only after Open — as daemon startup
// previously did — deleted legacy terminal rows under the built-in
// defaults before the configured retention could preserve them
// (issue #6248). Nonpositive values leave the built-in defaults in
// place, exactly as Configure does.
func OpenWithRetention(dbPath string, ttl, maxAge time.Duration) (*Store, error) {
	if err := os.MkdirAll(filepath.Dir(dbPath), 0755); err != nil {
		return nil, fmt.Errorf("creating task db dir: %w", err)
	}
	db, err := sql.Open("sqlite", dbPath+"?_pragma=busy_timeout(5000)&_pragma=journal_mode(WAL)")
	if err != nil {
		return nil, fmt.Errorf("opening task db: %w", err)
	}
	st := New()
	if ttl > 0 {
		st.ttl = ttl
	}
	if maxAge > 0 {
		st.maxAge = maxAge
	}
	st.db = db
	if err := st.initDB(); err != nil {
		_ = db.Close()
		return nil, err
	}
	return st, nil
}

func (s *Store) initDB() error {
	schema := `
CREATE TABLE IF NOT EXISTS tasks (
    task_id        TEXT PRIMARY KEY,
    status         TEXT NOT NULL,
    status_message TEXT,
    created_at_ms  INTEGER NOT NULL,
    updated_at_ms  INTEGER NOT NULL,
    ttl_ms         INTEGER NOT NULL,
    last_touched_ms INTEGER NOT NULL DEFAULT 0,
    expires_ms      INTEGER NOT NULL DEFAULT 0,
    poll_interval  INTEGER NOT NULL,
    result_json    TEXT,
    fault_json     TEXT
);`
	if _, err := s.db.Exec(schema); err != nil {
		return fmt.Errorf("initializing tasks schema: %w", err)
	}
	if err := ensureTaskColumn(s.db, "last_touched_ms"); err != nil {
		return err
	}
	if err := ensureTaskColumn(s.db, "expires_ms"); err != nil {
		return err
	}
	return s.loadTasksFromDB()
}

func ensureTaskColumn(db *sql.DB, name string) error {
	rows, err := db.Query("PRAGMA table_info(tasks)")
	if err != nil {
		return fmt.Errorf("reading tasks schema: %w", err)
	}
	defer func() { _ = rows.Close() }()
	for rows.Next() {
		var cid, notNull, primaryKey int
		var column, kind string
		var defaultValue sql.NullString
		if err := rows.Scan(&cid, &column, &kind, &notNull, &defaultValue, &primaryKey); err != nil {
			return fmt.Errorf("reading tasks schema row: %w", err)
		}
		if column == name {
			return nil
		}
	}
	if err := rows.Err(); err != nil {
		return fmt.Errorf("reading tasks schema rows: %w", err)
	}
	if err := rows.Close(); err != nil {
		return fmt.Errorf("closing tasks schema rows: %w", err)
	}
	if _, err := db.Exec("ALTER TABLE tasks ADD COLUMN " + name + " INTEGER NOT NULL DEFAULT 0"); err != nil {
		return fmt.Errorf("adding tasks.%s: %w", name, err)
	}
	return nil
}

func (s *Store) loadTasksFromDB() error {
	rows, err := s.db.Query(`
SELECT task_id, status, status_message, created_at_ms, updated_at_ms, ttl_ms, last_touched_ms, expires_ms, poll_interval, result_json, fault_json
FROM tasks`)
	if err != nil {
		return fmt.Errorf("querying stored tasks: %w", err)
	}
	defer func() { _ = rows.Close() }()

	now := time.Now()
	nowMs := now.UnixMilli()

	var toDelete []string
	var toUpdate []*Task

	for rows.Next() {
		var id, status, statusMsg string
		var createdMs, updatedMs, ttlMs, lastTouchedMs, expiresMs, pollInterval int64
		var resultJSON, faultJSON sql.NullString

		if err := rows.Scan(&id, &status, &statusMsg, &createdMs, &updatedMs, &ttlMs, &lastTouchedMs, &expiresMs, &pollInterval, &resultJSON, &faultJSON); err != nil {
			continue
		}

		createdAt := time.UnixMilli(createdMs)
		updatedAt := time.UnixMilli(updatedMs)

		t := &Task{
			TaskID:        id,
			Status:        status,
			StatusMessage: statusMsg,
			CreatedAt:     createdAt,
			LastUpdatedAt: updatedAt,
			TTL:           ttlMs,
			PollInterval:  pollInterval,
			done:          make(chan struct{}),
		}
		lastTouched := time.UnixMilli(lastTouchedMs)
		if lastTouchedMs <= 0 {
			lastTouched = updatedAt
			if !Terminal(status) {
				lastTouched = createdAt
			}
		}
		t.LastTouched = &lastTouched
		needsUpdate := false

		if resultJSON.Valid && resultJSON.String != "" {
			var res any
			if json.Unmarshal([]byte(resultJSON.String), &res) == nil {
				t.result = res
			}
		}
		if faultJSON.Valid && faultJSON.String != "" {
			var fault Fault
			if json.Unmarshal([]byte(faultJSON.String), &fault) == nil {
				t.fault = &fault
			}
		}

		if !Terminal(t.Status) {
			t.Status = Failed
			t.StatusMessage = "daemon restarted before task completion"
			t.fault = &Fault{Code: -32603, Message: t.StatusMessage}
			t.LastUpdatedAt = now
			lastTouched = now
			t.LastTouched = &lastTouched
			needsUpdate = true
		}

		// Retention runs from the terminal transition, which the row's
		// updated_at_ms records; a row that was still working at shutdown
		// became terminal just above, at load time (its LastUpdatedAt).
		expiresAt := time.UnixMilli(expiresMs)
		if expiresMs <= 0 {
			expiresAt = deadline(t.LastUpdatedAt, *t.LastTouched, time.Duration(ttlMs)*time.Millisecond, s.maxAge)
			t.ExpiresAt = &expiresAt
			needsUpdate = true
		} else {
			t.ExpiresAt = &expiresAt
		}
		expiresAtMs := expiresAt.UnixMilli()
		if expiresAtMs <= nowMs {
			toDelete = append(toDelete, id)
			continue
		}

		close(t.done)
		t.doneClosed = true
		s.tasks[t.TaskID] = t
		t.refreshTTLRemaining(now)
		s.armExpiryLocked(t, time.Duration(expiresAtMs-nowMs)*time.Millisecond)
		if needsUpdate {
			toUpdate = append(toUpdate, t)
		}
	}

	for _, id := range toDelete {
		s.deleteFromDBLocked(id)
	}
	for _, t := range toUpdate {
		s.saveTaskLocked(t)
	}
	return nil
}

func (s *Store) saveTaskLocked(t *Task) {
	if s.db == nil {
		return
	}
	var resJSON, faultJSON *string
	if t.result != nil {
		if b, err := json.Marshal(t.result); err == nil {
			str := string(b)
			resJSON = &str
		}
	}
	if t.fault != nil {
		if b, err := json.Marshal(t.fault); err == nil {
			str := string(b)
			faultJSON = &str
		}
	}

	query := `
INSERT INTO tasks (task_id, status, status_message, created_at_ms, updated_at_ms, ttl_ms, last_touched_ms, expires_ms, poll_interval, result_json, fault_json)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(task_id) DO UPDATE SET
  status = excluded.status,
  status_message = excluded.status_message,
  updated_at_ms = excluded.updated_at_ms,
  last_touched_ms = excluded.last_touched_ms,
  expires_ms = excluded.expires_ms,
  result_json = excluded.result_json,
  fault_json = excluded.fault_json;`

	_, _ = s.db.Exec(query,
		t.TaskID,
		t.Status,
		t.StatusMessage,
		t.CreatedAt.UnixMilli(),
		t.LastUpdatedAt.UnixMilli(),
		t.TTL,
		unixMilli(t.LastTouched),
		unixMilli(t.ExpiresAt),
		t.PollInterval,
		resJSON,
		faultJSON,
	)
}

func unixMilli(value *time.Time) int64 {
	if value == nil {
		return 0
	}
	return value.UnixMilli()
}

func (s *Store) deleteFromDBLocked(id string) {
	if s.db == nil {
		return
	}
	_, _ = s.db.Exec("DELETE FROM tasks WHERE task_id = ?", id)
}

func (s *Store) Close() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.db != nil {
		return s.db.Close()
	}
	return nil
}

func (s *Store) SetTaskCancel(id string, fn func(ctx context.Context, reason string)) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if t, ok := s.tasks[id]; ok {
		t.OnCancel = fn
	}
}

func (s *Store) ensure() {
	if s.tasks == nil {
		s.tasks = map[string]*Task{}
	}
}

// NewID is the handle a caller holds a task by: 128 bits of randomness
// (16 bytes), so an id is unguessable at the scale the per-requestor caps
// now let the store grow to.
func NewID() string {
	var b [16]byte
	_, _ = RandSource.Read(b[:])
	return "tsk-" + hex.EncodeToString(b[:])
}

// Requestor is the composite key a task counts against for the store's
// per-requestor limits: a ROLE and an IDENTITY, as separate fields —
// never a plain string, and never distinguished by a string prefix.
//
// The role field exists because one logical operation can create two
// store tasks in different capacities: on the daemon's embedded MCP
// surface a task-supporting call creates the /v1/ask backing (role
// ask) and the MCP wrapper (role caller), and for a client-named
// identity both carried the same plain string, so a per-requestor cap
// of 1 refused — and abandoned — the operation, and the default cap
// silently halved. Separate roles keep one operation to one slot in
// each ledger (Codex + CodeRabbit review, PR #2869).
//
// The identity field is caller-supplied text (a session id, a run id,
// an MCP identity key). Because the role is a separate field, no
// caller-supplied identity can collide with a reserved bucket: a
// session literally named "unbound" keys as {caller, "unbound"},
// which is not the unbound bucket {, } (CodeRabbit review, PR #2869).
type Requestor struct {
	Role     string
	Identity string
}

// The requestor roles. RoleCaller is an ordinary creation site
// charging its caller; RoleAsk is the ask seam (/v1/ask backings,
// handleAskBegin) charging in its asking capacity; RoleReport is the
// reporting-call seam (handleCallReporting) charging in its reporting
// capacity, separate from the execution wrappers that spawn it.
const (
	RoleCaller = "caller"
	RoleAsk    = "ask"
	RoleReport = "report"
)

// Caller names an ordinary requestor: the given caller identity in
// the caller role. An empty identity is the unbound bucket.
func Caller(identity string) Requestor {
	if identity == "" {
		return UnboundRequestor
	}
	return Requestor{Role: RoleCaller, Identity: identity}
}

// Ask names an ask-seam requestor: the given caller identity in the
// ask role. An empty identity is the anonymous ask bucket.
func Ask(identity string) Requestor {
	return Requestor{Role: RoleAsk, Identity: identity}
}

// Report names a reporting-call requestor: the given caller identity
// in the report role. An empty identity is the anonymous report
// bucket.
func Report(identity string) Requestor {
	return Requestor{Role: RoleReport, Identity: identity}
}

// UnboundRequestor is the bucket a creation site passes when it holds
// no caller identity. It is capped like any other requestor — "we
// don't know who asked" must not mean "unlimited".
var UnboundRequestor = Requestor{}

// AskRequestor is the bucket anonymous ask-seam tasks count against.
// It is its own bucket, not UnboundRequestor, for the two-ledgers
// reason recorded on Requestor.
var AskRequestor = Requestor{Role: RoleAsk}

// ReportRequestor is the bucket anonymous reporting-call tasks count
// against. It is its own bucket, not UnboundRequestor, for the
// two-ledgers reason recorded on Requestor: a /v1/exec wrapper task
// (caller role) stays non-terminal for the whole run while the
// script's tool calls arrive as reporting tasks under the same
// session identity, so charging both to one ledger lets the wrapper
// starve its own calls -- at a per-requestor cap of 1 the first
// reporting call is refused outright (Codex review, PR #2869).
var ReportRequestor = Requestor{Role: RoleReport}

// key is the window-counter spelling of the requestor: the two fields
// joined by a separator neither can be confused across, because the
// role is one of a fixed set spelled first.
func (r Requestor) key() string { return r.Role + "\x00" + r.Identity }

// The task-store limit defaults (protoTasks.*). A seam wires the settings
// over these; the store applies them when its fields are unset.
const (
	DefaultMaxTTL          = time.Hour
	DefaultMaxPerRequestor = 32
	DefaultPerMinute       = 120
)

// DefaultCancelGrace is declared with the defaults (protoTasks.cancelGrace),
// not here: the defaults guard forbids a package-level redeclaration.
var DefaultCancelGrace = defaults.TaskCancelGrace

// ErrPerRequestorCap is the named sentinel Start returns when a requestor
// already holds MaxPerRequestor non-terminal tasks. The seams map it to
// -32603 (MCP) / 429 (/v1); the wrapped message names the limit.
var ErrPerRequestorCap = errors.New("tasks: per-requestor task cap reached")

// ErrRateWindow is the named sentinel Admit reports through the seams when
// a requestor exceeds the tasks/* per-minute window: -32603 / 429, with
// protoTasks.perMinute named.
var ErrRateWindow = errors.New("tasks: protoTasks.perMinute admission window exceeded")

// Window is a small fixed-window admission counter, one bucket per key.
// It exists for the tasks/* seams (internal/tasks is the one package both
// surfaces share); it is not a general rate-limit framework.
type Window struct {
	mu     sync.Mutex
	limit  int
	counts map[string]*windowBucket
	// lastSweep is when expired buckets were last removed; sweeping on
	// every new key made admission O(N^2) over distinct requestors.
	lastSweep time.Time
}

type windowBucket struct {
	start time.Time
	n     int
}

// NewWindow returns a Window admitting at most limit calls per key per
// minute.
func NewWindow(limit int) *Window {
	return &Window{limit: limit, counts: map[string]*windowBucket{}}
}

// Allow reports whether one more call for key is admitted in the current
// fixed minute window, and records it when it is.
func (w *Window) Allow(key string, now time.Time) bool {
	w.mu.Lock()
	defer w.mu.Unlock()
	b, ok := w.counts[key]
	if !ok || now.Sub(b.start) >= time.Minute {
		// Sweep at most once per minute so admission stays O(1) per new
		// key; the sweep still bounds the map: distinct requestors on a
		// long-lived daemon cannot accumulate entries past their windows.
		if now.Sub(w.lastSweep) >= time.Minute {
			for k, v := range w.counts {
				if now.Sub(v.start) >= time.Minute {
					delete(w.counts, k)
				}
			}
			w.lastSweep = now
		}
		w.counts[key] = &windowBucket{start: now, n: 1}
		return true
	}
	if b.n >= w.limit {
		return false
	}
	b.n++
	return true
}

// Admit counts one tasks/* call for requestor against the store's
// per-minute window (PerMinute). Every tasks/* method counts, tasks/result
// included: a long poll is admitted once and held server-side, so counting
// at admission bounds the call rate without punishing the wait — and an
// exemption would leave one method permanently unwindowed (triage
// settlement, task 3733).
func (s *Store) Admit(requestor Requestor) bool {
	limit := s.PerMinute
	if limit == 0 {
		limit = DefaultPerMinute
	}
	if limit < 0 {
		return true // window disabled (protoTasks.perMinute = 0)
	}
	s.mu.Lock()
	if s.win == nil || s.winLimit != limit {
		s.win = NewWindow(limit)
		s.winLimit = limit
	}
	win := s.win
	s.mu.Unlock()
	allowed := win.Allow(requestor.key(), time.Now())
	if !allowed {
		attrs := requestorAuditAttrs(requestor)
		attrs["limit"] = "per_minute"
		attrs["limit_value"] = limit
		s.audit(slog.LevelInfo, "task.limit_refused", attrs)
	}
	return allowed
}

// DefaultTTL is the retention a caller gets when it does not ask for one, in
// milliseconds.
func DefaultTTL() int64 { return int64(defaults.TaskTTL / time.Millisecond) }

// armExpiryLocked (re)starts t's retention timer: after d, a task that
// is still in the store and terminal is deleted from memory and the DB.
// A task that is not terminal is never deleted by this timer — retention
// is the only clock it serves. Called with s.mu held, or during load
// before the store is shared.
func (s *Store) armExpiryLocked(t *Task, d time.Duration) {
	if t.timer != nil {
		t.timer.Stop()
	}
	if d <= 0 {
		return
	}
	id := t.TaskID
	t.timer = time.AfterFunc(d, func() {
		s.mu.Lock()
		defer s.mu.Unlock()
		cur, ok := s.tasks[id]
		if !ok || cur != t || !Terminal(cur.Status) {
			return
		}
		if cur.ExpiresAt != nil && time.Now().Before(*cur.ExpiresAt) {
			s.armExpiryLocked(cur, time.Until(*cur.ExpiresAt))
			return
		}
		delete(s.tasks, id)
		s.deleteFromDBLocked(id)
	})
}

// armRetentionLocked starts t's full retention clock (its TTL) from now.
// Called with s.mu held, at the moment t reaches a terminal state.
func (s *Store) armRetentionLocked(t *Task) {
	now := t.LastUpdatedAt
	t.LastTouched = &now
	expires := deadline(t.LastUpdatedAt, now, time.Duration(t.TTL)*time.Millisecond, s.maxAge)
	t.ExpiresAt = &expires
	t.refreshTTLRemaining(time.Now())
	s.armExpiryLocked(t, time.Until(expires))
}

// Start runs fn in the background and returns its handle at once.
//
// The requestor key is who the task counts against: Start clamps the
// requested TTL to the store's MaxTTL and refuses with ErrPerRequestorCap
// when the requestor already holds MaxPerRequestor non-terminal tasks. The
// empty key is the UnboundRequestor bucket, capped like any other.
//
// The ttl is retention, not a deadline: it is measured from when the task
// reaches a terminal state (see Task.TTL), so work that outlives its
// ttl still runs to completion. A caller that needs the work itself
// bounded uses StartWithDeadline.
func (s *Store) Start(requestor Requestor, ttl int64, fn func(ctx context.Context) (any, *Fault)) (Task, error) {
	return s.StartWithDeadline(requestor, ttl, 0, fn)
}

// StartWithDeadline is Start with an explicit bound on the work itself:
// a task still not terminal deadline after it starts is cancelled, as
// if Cancel had been called, and its retention clock starts from that
// cancellation. A zero deadline leaves the work unbounded. The
// requestor key, TTL clamp and per-requestor cap are Start's, above.
//
// A deadline task's retention is floored at two poll intervals, under
// the clamp: both deadline seams in the tree are internal tasks whose
// result a wrapper collects by polling at the interval the task itself
// advertises (the daemon-backed ask path: driveAsked polls, sleeping a
// full TaskPoll between looks while a question is open). A clamped TTL
// below one poll cycle -- protoTasks.maxTTL=100ms against a 1s poll --
// would delete a finished backing result before the wrapper's next
// look, and its poll would meet a 404 and record a successful call as
// failed (Codex on #2869, round 6). The floor is on the returned task's
// TTL, so what a client is told is what the store does.
func (s *Store) StartWithDeadline(requestor Requestor, ttl int64, deadline time.Duration, fn func(ctx context.Context) (any, *Fault)) (Task, error) {
	if ttl <= 0 {
		s.mu.Lock()
		configuredTTL := s.ttl
		s.mu.Unlock()
		if configuredTTL <= 0 {
			ttl = DefaultTTL()
		} else {
			ttl = configuredTTL.Milliseconds()
		}
	}
	ttlClamped := false
	maxTTL := s.MaxTTL
	if maxTTL <= 0 {
		maxTTL = DefaultMaxTTL
	}
	if ttl > maxTTL.Milliseconds() {
		ttl = maxTTL.Milliseconds()
		ttlClamped = true
	}
	if deadline > 0 {
		poll := s.PollInterval
		if poll <= 0 {
			poll = defaults.TaskPollInterval
		}
		if floor := (2 * poll).Milliseconds(); ttl < floor {
			ttl = floor
		}
	}
	capN := s.MaxPerRequestor
	if capN <= 0 {
		capN = DefaultMaxPerRequestor
	}
	s.mu.Lock()
	s.ensure()
	live := 0
	for _, ot := range s.tasks {
		if ot.requestor == requestor && !Terminal(ot.Status) {
			live++
		}
	}
	if live >= capN {
		// Counted and inserted under the one lock: concurrent bursts for
		// the same requestor cannot each observe the same free slot.
		s.mu.Unlock()
		attrs := requestorAuditAttrs(requestor)
		attrs["limit"] = "max_per_requestor"
		attrs["limit_value"] = capN
		s.audit(slog.LevelInfo, "task.limit_refused", attrs)
		return Task{}, fmt.Errorf("%w: protoTasks.maxPerRequestor allows %d non-terminal tasks per requestor", ErrPerRequestorCap, capN)
	}
	now := time.Now()
	ctx, cancel := context.WithCancel(context.Background())
	poll := s.PollInterval
	if poll <= 0 {
		poll = defaults.TaskPollInterval
	}
	t := &Task{
		TaskID: NewID(), Status: Working,
		CreatedAt: now, LastUpdatedAt: now,
		LastTouched: &now,
		TTL:         ttl, PollInterval: poll.Milliseconds(),
		requestor: requestor,
		cancel:    cancel, done: make(chan struct{}),
	}
	ctx = context.WithValue(ctx, idKey{}, t.TaskID)
	s.tasks[t.TaskID] = t
	s.saveTaskLocked(t)
	s.mu.Unlock()
	attrs := requestorAuditAttrs(requestor)
	attrs["task_id"] = t.TaskID
	attrs["effective_ttl_ms"] = ttl
	attrs["ttl_clamped"] = ttlClamped
	s.audit(slog.LevelInfo, "task.created", attrs)

	go func() {
		result, fault := fn(ctx)
		moved := s.finish(t, result, fault)
		s.closeDone(t)
		if moved {
			s.changed(t.TaskID)
		}
	}()

	if deadline > 0 {
		// The deadline bounds the work, never the retention: firing
		// cancels a still-running task through the ordinary cancel
		// path, and retention runs from that terminal transition.
		time.AfterFunc(deadline, func() {
			s.CancelWithReason(t.TaskID, "deadline exceeded before the task completed")
		})
	}
	// Snapshotted under the lock, so the value handed back cannot be read
	// while the goroutine above is writing the original.
	s.mu.Lock()
	defer s.mu.Unlock()
	return *t, nil
}

// finish records what a task's body produced, and reports whether that
// changed its status: a task cancelled while running keeps its cancellation.
func (s *Store) finish(t *Task, result any, fault *Fault) bool {
	s.mu.Lock()
	if Terminal(t.Status) {
		s.mu.Unlock()
		return false // the first terminal recording under the lock wins
	}
	wasCancelling := t.cancelling
	t.cancelling = false
	if t.cancelTimer != nil {
		t.cancelTimer.Stop()
		t.cancelTimer = nil
	}
	t.LastUpdatedAt = time.Now()
	t.InputRequests = nil
	if fault != nil {
		t.Status, t.fault = Failed, fault
		t.StatusMessage = fault.Message
		if t.StatusMessage == "" && wasCancelling {
			t.StatusMessage = "completed during cancellation"
		}
	} else {
		t.Status, t.result = Completed, result
		// A tool result that is itself an error is a failed task, per the
		// specification's own note on TaskStatus.
		if m, ok := result.(map[string]any); ok {
			if isErr, _ := m["isError"].(bool); isErr {
				t.Status = Failed
				t.StatusMessage = toolResultErrorMessage(m)
			}
		}
		if t.StatusMessage == "" && wasCancelling {
			t.StatusMessage = "completed during cancellation"
		}
	}
	s.armRetentionLocked(t)
	s.saveTaskLocked(t)
	s.mu.Unlock()
	s.logTerminal(t)
	return true
}

// List returns a snapshot, oldest first.
func (s *Store) List() []Task {
	s.mu.Lock()
	defer s.mu.Unlock()
	out := make([]Task, 0, len(s.tasks))
	for _, t := range s.tasks {
		out = append(out, snapshot(t, time.Now()))
	}
	sort.Slice(out, func(i, j int) bool { return out[i].CreatedAt.Before(out[j].CreatedAt) })
	return out
}

// Get returns one task's current state.
func (s *Store) Get(id string) (Task, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	t, ok := s.tasks[id]
	if !ok {
		return Task{}, false
	}
	return snapshot(t, time.Now()), true
}

// Read returns a public task item snapshot and renews only terminal-result
// retention. Collection and internal reads use Get/List instead.
func (s *Store) Read(id string) (Task, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	t, ok := s.tasks[id]
	if !ok || !s.touchLocked(t, time.Now()) {
		return Task{}, false
	}
	return snapshot(t, time.Now()), true
}

func (s *Store) touchLocked(t *Task, now time.Time) bool {
	if Terminal(t.Status) {
		if t.ExpiresAt == nil {
			s.armRetentionLocked(t)
		}
		if !now.Before(*t.ExpiresAt) {
			if t.timer != nil {
				t.timer.Stop()
			}
			delete(s.tasks, t.TaskID)
			s.deleteFromDBLocked(t.TaskID)
			return false
		}
		expires := deadline(t.LastUpdatedAt, now, time.Duration(t.TTL)*time.Millisecond, s.maxAge)
		if expires.Before(*t.ExpiresAt) {
			expires = *t.ExpiresAt
		}
		t.ExpiresAt = &expires
		t.LastTouched = &now
		s.armExpiryLocked(t, time.Until(expires))
	} else {
		t.LastTouched = &now
	}
	t.refreshTTLRemaining(now)
	s.saveTaskLocked(t)
	return true
}

// SetStatus lets a running call report progress, and is how a call waiting
// on an elicitation shows input_required.
func (s *Store) SetStatus(id, status, message string) {
	s.mu.Lock()
	t, ok := s.tasks[id]
	moved := ok && !t.cancelling && !Terminal(t.Status) && (t.Status != status || t.StatusMessage != message)
	if moved {
		t.Status, t.StatusMessage, t.LastUpdatedAt = status, message, time.Now()
		if Terminal(status) {
			s.armRetentionLocked(t)
		}
		s.saveTaskLocked(t)
	}
	s.mu.Unlock()
	if moved {
		s.changed(id)
		if Terminal(status) {
			if snap, ok := s.Get(id); ok {
				s.logTerminal(&snap)
			}
		}
	}
}

// SetInput records the questions a running task is waiting on. A non-empty
// set makes it input_required, an empty one working again. Called with the
// same set it changes nothing, lastUpdatedAt included.
func (s *Store) SetInput(id string, requests map[string]any) {
	if s.setInput(id, requests) {
		s.changed(id)
	}
}

func (s *Store) setInput(id string, requests map[string]any) bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	t, ok := s.tasks[id]
	if !ok || t.cancelling || Terminal(t.Status) {
		return false
	}
	status := Working
	if len(requests) > 0 {
		status = InputRequired
	} else {
		requests = nil
	}
	if status == t.Status && sameKeys(requests, t.InputRequests) {
		return false
	}
	t.Status, t.InputRequests, t.LastUpdatedAt = status, requests, time.Now()
	s.saveTaskLocked(t)
	return true
}

func sameKeys(a, b map[string]any) bool {
	if len(a) != len(b) {
		return false
	}
	for k := range a {
		if _, ok := b[k]; !ok {
			return false
		}
	}
	return true
}

// Cancel stops a task and returns its final state.
func (s *Store) Cancel(id string) (Task, bool) {
	return s.CancelWithReason(id, "cancelled")
}

// CancelWithReason begins cooperative cancellation and returns the current
// wire-visible state. If the work finishes within CancelGrace, its own
// terminal status wins; otherwise the store records Cancelled with reason.
func (s *Store) CancelWithReason(id string, reason string) (Task, bool) {
	s.mu.Lock()
	t, ok := s.tasks[id]
	if !ok {
		s.mu.Unlock()
		return Task{}, false
	}
	start := !Terminal(t.Status) && !t.cancelling
	var onCancel func(ctx context.Context, reason string)
	var cancel context.CancelFunc
	if start {
		t.cancelling = true
		grace := s.CancelGrace
		if grace <= 0 {
			grace = DefaultCancelGrace
		}
		t.cancelTimer = time.AfterFunc(grace, func() {
			s.expireCancellation(t, reason)
		})
		onCancel = t.OnCancel
		cancel = t.cancel
	}
	snap := *t
	hook := s.OnCancelHook
	s.mu.Unlock()

	if start {
		if onCancel != nil {
			ctx, cancelCtx := context.WithTimeout(context.Background(), defaults.UpstreamCancelSendTimeout)
			onCancel(ctx, reason)
			cancelCtx()
		}
		if hook != nil {
			hook(id, reason)
		}
		if cancel != nil {
			cancel()
		}
	}
	return snap, true
}

// CancelImmediate records Cancelled at once and returns the terminal
// snapshot. It is the legacy 2025-11-25 tasks/cancel semantics
// (conformance: tasks-cancel-transition-before-response,
// tasks-cancelled-sticky): the transition happens before the reply and
// stands whatever the work does afterwards. The cooperative grace
// window of CancelWithReason belongs to the modern extension path
// only. The work is still notified (OnCancel, the store hook, and its
// context), it just cannot change the recorded ending.
func (s *Store) CancelImmediate(id string, reason string) (Task, bool) {
	s.mu.Lock()
	t, ok := s.tasks[id]
	if !ok {
		s.mu.Unlock()
		return Task{}, false
	}
	transitioned := !Terminal(t.Status)
	var onCancel func(ctx context.Context, reason string)
	var cancel context.CancelFunc
	if transitioned {
		if t.cancelTimer != nil {
			t.cancelTimer.Stop()
			t.cancelTimer = nil
		}
		t.cancelling = false
		t.Status, t.LastUpdatedAt = Cancelled, time.Now()
		if t.StatusMessage == "" && reason != "" {
			t.StatusMessage = reason
		}
		s.armRetentionLocked(t)
		s.saveTaskLocked(t)
		onCancel = t.OnCancel
		cancel = t.cancel
	}
	snap := *t
	hook := s.OnCancelHook
	s.mu.Unlock()

	if transitioned {
		s.logTerminal(t)
		if onCancel != nil {
			ctx, cancelCtx := context.WithTimeout(context.Background(), defaults.UpstreamCancelSendTimeout)
			onCancel(ctx, reason)
			cancelCtx()
		}
		if hook != nil {
			hook(id, reason)
		}
		if cancel != nil {
			cancel()
		}
		// Notify watchers last: a watcher may wait on the task's
		// completion (Store.Result waits for done), which only the
		// now-cancelled work can deliver -- notifying first could
		// deadlock the watcher against the pending cancellation.
		s.changed(id)
	}
	return snap, true
}

// closeDoneLocked releases Result waiters exactly once. The caller
// holds s.mu. Terminal transitions recorded without the work's return
// (grace expiry) close done themselves; the worker goroutine's close
// after finish is then a no-op instead of a double-close panic.
func (s *Store) closeDoneLocked(t *Task) {
	if t.doneClosed {
		return
	}
	t.doneClosed = true
	close(t.done)
}

// closeDone is closeDoneLocked for callers not holding s.mu.
func (s *Store) closeDone(t *Task) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.closeDoneLocked(t)
}

// expireCancellation records the cancel only if the task's work did not
// record a terminal status first. Called by the per-task grace timer.
func (s *Store) expireCancellation(t *Task, reason string) {
	s.mu.Lock()
	if s.tasks[t.TaskID] != t || !t.cancelling || Terminal(t.Status) {
		s.mu.Unlock()
		return
	}
	t.cancelling = false
	t.cancelTimer = nil
	t.Status, t.LastUpdatedAt = Cancelled, time.Now()
	if t.StatusMessage == "" && reason != "" {
		t.StatusMessage = reason
	}
	s.armRetentionLocked(t)
	s.saveTaskLocked(t)
	// The work may never return (that is the path grace expiry exists
	// for), so release Result waiters here: they read the recorded
	// Cancelled status and return the cancelled fault (#6897).
	s.closeDoneLocked(t)
	s.mu.Unlock()
	s.logTerminal(t)
	s.changed(t.TaskID)
}

// toolResultErrorMessage returns the first text content from an isError
// result, bounded by the protocol's status-message limit.
func toolResultErrorMessage(result map[string]any) string {
	if blocks, ok := result["content"].([]any); ok {
		for _, block := range blocks {
			part, ok := block.(map[string]any)
			if !ok || part["type"] != "text" {
				continue
			}
			text, ok := part["text"].(string)
			if !ok || text == "" {
				continue
			}
			runes := []rune(text)
			if len(runes) > 512 {
				runes = runes[:512]
			}
			return string(runes)
		}
	}
	return "tool returned an error result"
}

// ErrNoTask is returned for a handle the store does not hold, which usually
// means it expired rather than that it never existed.
type ErrNoTask struct{ ID string }

func (e ErrNoTask) Error() string { return "no task " + e.ID + "; it may have expired" }

// Result waits for a task to finish and returns what it produced.
//
// Blocking is the point: this is the "wait for it" half of the pair, and Get
// is the "check on it" half. The context bounds the wait, so a caller that
// waited long enough can stop without stopping the task.
func (s *Store) Result(ctx context.Context, id string) (any, *Fault, error) {
	s.mu.Lock()
	t, ok := s.tasks[id]
	s.mu.Unlock()
	if !ok {
		return nil, nil, ErrNoTask{ID: id}
	}
	select {
	case <-t.done:
	case <-ctx.Done():
		return nil, nil, ctx.Err()
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	cur, stillPresent := s.tasks[id]
	if !stillPresent || cur != t || (t.ExpiresAt != nil && !time.Now().Before(*t.ExpiresAt)) {
		if stillPresent && cur == t {
			if t.timer != nil {
				t.timer.Stop()
			}
			delete(s.tasks, id)
			s.deleteFromDBLocked(id)
		}
		return nil, nil, ErrNoTask{ID: id}
	}
	if t.fault != nil {
		return nil, t.fault, nil
	}
	if t.Status == Cancelled {
		return nil, &Fault{Code: -32603, Message: "the task was cancelled"}, nil
	}
	return t.result, nil, nil
}

// ReadResult is the public result-read path. Result remains non-touching for
// internal polling and notification machinery.
func (s *Store) ReadResult(ctx context.Context, id string) (any, *Fault, error) {
	result, fault, err := s.Result(ctx, id)
	if err != nil {
		return nil, nil, err
	}
	s.mu.Lock()
	t, ok := s.tasks[id]
	if !ok || !s.touchLocked(t, time.Now()) {
		s.mu.Unlock()
		return nil, nil, ErrNoTask{ID: id}
	}
	s.mu.Unlock()
	return result, fault, nil
}
