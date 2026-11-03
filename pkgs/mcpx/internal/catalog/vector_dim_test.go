package catalog

import (
	"bytes"
	"database/sql"
	"encoding/json"
	"errors"
	"path/filepath"
	"strings"
	"testing"

	_ "modernc.org/sqlite"
)

// The dimension contract (issue #1643): a vector whose length is not
// VectorDim must never enter the index silently — Put/Import reject it
// with an error naming expected vs actual dimensions, and loadFromDB
// surfaces rows it cannot load instead of dropping them quietly.

func TestPutRejectsNonIndexDimension(t *testing.T) {
	idx, err := NewVectorIndex("")
	if err != nil {
		t.Fatalf("NewVectorIndex: %v", err)
	}

	err = idx.Put("tool.remote", make(Vector, 1536))
	if err == nil {
		t.Fatal("Put of a 1536-dimensional vector returned nil error")
	}
	var dimErr *DimensionError
	if !errors.As(err, &dimErr) {
		t.Fatalf("Put error type = %T, want *DimensionError (%v)", err, err)
	}
	if dimErr.Expected != VectorDim || dimErr.Actual != 1536 {
		t.Fatalf("DimensionError = %+v, want Expected=%d Actual=1536", dimErr, VectorDim)
	}
	if !strings.Contains(err.Error(), "384") || !strings.Contains(err.Error(), "1536") {
		t.Fatalf("error %q must name expected and actual dimensions", err)
	}
	if _, ok := idx.Get("tool.remote"); ok {
		t.Fatal("rejected vector was stored in the index")
	}

	good := Embed("inspect web page contents")
	if err := idx.Put("tool.local", good); err != nil {
		t.Fatalf("Put of a %d-dimensional vector: %v", VectorDim, err)
	}
	if _, ok := idx.Get("tool.local"); !ok {
		t.Fatal("valid vector was not stored")
	}
}

func TestPutNonIndexDimensionNotPersistedAcrossRestart(t *testing.T) {
	dbPath := filepath.Join(t.TempDir(), "vectors.db")
	idx, err := NewVectorIndex(dbPath)
	if err != nil {
		t.Fatalf("NewVectorIndex: %v", err)
	}
	if err := idx.Put("tool.remote", make(Vector, 1536)); err == nil {
		t.Fatal("Put of a 1536-dimensional vector returned nil error")
	}
	if err := idx.Close(); err != nil {
		t.Fatalf("Close: %v", err)
	}

	idx2, err := NewVectorIndex(dbPath)
	if err != nil {
		t.Fatalf("reopen: %v", err)
	}
	defer idx2.Close()
	if _, ok := idx2.Get("tool.remote"); ok {
		t.Fatal("non-index-dimension vector survived a restart")
	}
	if got := idx2.DroppedOnLoad(); got != 0 {
		t.Fatalf("DroppedOnLoad = %d, want 0 (the write failed loudly, nothing was persisted)", got)
	}
}

func TestImportRejectsNonIndexDimension(t *testing.T) {
	idx, err := NewVectorIndex("")
	if err != nil {
		t.Fatalf("NewVectorIndex: %v", err)
	}

	payload := ExportedEmbeddings{
		Version: 1,
		Model:   "remote-embedder",
		Dim:     1536,
		Vectors: map[string][]float32{
			"tool.local":  make([]float32, VectorDim),
			"tool.remote": make([]float32, 1536),
		},
	}
	var buf bytes.Buffer
	if err := json.NewEncoder(&buf).Encode(payload); err != nil {
		t.Fatalf("encode payload: %v", err)
	}
	count, err := idx.Import(&buf)
	if err == nil {
		t.Fatal("Import carrying a 1536-dimensional vector returned nil error")
	}
	if !strings.Contains(err.Error(), "384") || !strings.Contains(err.Error(), "1536") {
		t.Fatalf("Import error %q must name expected and actual dimensions", err)
	}
	if count != 1 {
		t.Fatalf("Import count = %d, want 1 (only the valid vector)", count)
	}
	if _, ok := idx.Get("tool.local"); !ok {
		t.Fatal("valid vector from the payload was not imported")
	}
	if _, ok := idx.Get("tool.remote"); ok {
		t.Fatal("dimension-mismatched vector was imported")
	}
}

func TestLoadFromDBSurfacesDroppedRows(t *testing.T) {
	dbPath := filepath.Join(t.TempDir(), "vectors.db")
	idx, err := NewVectorIndex(dbPath)
	if err != nil {
		t.Fatalf("NewVectorIndex: %v", err)
	}
	idx.IndexTool("tool.local", "inspect web page contents")
	if err := idx.Close(); err != nil {
		t.Fatalf("Close: %v", err)
	}

	// Simulate a row persisted before the dimension contract (or by a
	// foreign writer): a blob that is not a VectorDim vector.
	db, err := sql.Open("sqlite", dbPath)
	if err != nil {
		t.Fatalf("open db: %v", err)
	}
	if _, err := db.Exec(`INSERT OR REPLACE INTO tool_embeddings (key, vector) VALUES (?, ?)`, "tool.foreign", make([]byte, 100)); err != nil {
		t.Fatalf("insert foreign row: %v", err)
	}
	if err := db.Close(); err != nil {
		t.Fatalf("close db: %v", err)
	}

	idx2, err := NewVectorIndex(dbPath)
	if err != nil {
		t.Fatalf("reopen: %v", err)
	}
	defer idx2.Close()
	if _, ok := idx2.Get("tool.foreign"); ok {
		t.Fatal("foreign-dimension row was loaded into the index")
	}
	if got := idx2.DroppedOnLoad(); got != 1 {
		t.Fatalf("DroppedOnLoad = %d, want 1 (the unloadable row must be surfaced, not silently dropped)", got)
	}
	if _, ok := idx2.Get("tool.local"); !ok {
		t.Fatal("valid persisted vector was not reloaded")
	}
}
