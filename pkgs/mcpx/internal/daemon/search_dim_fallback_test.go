package daemon

import (
	"bytes"
	"context"
	"log"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/catalog"
)

// mismatchedDimEmbedder simulates a remote embedder whose dimension does
// not match the vector index dimension (e.g. text-embedding-3-small at
// 1536 dims against the 384-dim index).
type mismatchedDimEmbedder struct{ dim int }

func (m mismatchedDimEmbedder) Embed(_ context.Context, _ string) (catalog.Vector, error) {
	vec := make(catalog.Vector, m.dim)
	for i := range vec {
		vec[i] = 0.02
	}
	return vec, nil
}

func (m mismatchedDimEmbedder) Backend() string { return "mismatched-dim-test" }

// Regression test for issue #1643: when the configured embedder returns
// vectors whose dimension differs from the index dimension, semantic
// search must not silently score every pair 0 (CosineSimilarity on
// mismatched lengths) or cache vectors the index cannot reload. It logs
// the mismatch and falls back to the local embedder, for the query
// vector and for per-tool vectors alike.
func TestSearchSemanticDimensionMismatchFallsBack(t *testing.T) {
	var logBuf bytes.Buffer
	log.SetOutput(&logBuf)
	defer log.SetOutput(os.Stderr)

	idx, err := catalog.NewVectorIndex(filepath.Join(t.TempDir(), "vectors.db"))
	if err != nil {
		t.Fatalf("NewVectorIndex: %v", err)
	}
	defer idx.Close()

	reg := &Registry{
		testTools: []ToolInfo{
			{Namespace: "chrome_devtools", Tool: "click", Function: "chrome_devtools.click", Description: "Click an element on the active page"},
			{Namespace: "chrome_devtools", Tool: "take_snapshot", Function: "chrome_devtools.take_snapshot", Description: "Inspect web page contents and DOM elements"},
			{Namespace: "git", Tool: "commit", Function: "git.commit", Description: "Record changes to the repository"},
		},
		embedder:  mismatchedDimEmbedder{dim: 1536},
		vectorIdx: idx,
	}

	results := reg.SearchSemantic("inspect web page contents", 10, true)
	if len(results) == 0 {
		t.Fatal("expected results from semantic search with a mismatched embedder, got none")
	}
	if results[0].Tool != "take_snapshot" {
		t.Fatalf("expected top result to be take_snapshot, got %s", results[0].Tool)
	}

	// The per-tool fallback vector is what gets cached: it must be an
	// index-dimension vector, not the embedder's 1536-dim vector (which
	// Put would reject and a restart could never reload).
	vec, ok := idx.Get("chrome_devtools.take_snapshot")
	if !ok {
		t.Fatal("expected the searched tool vector to be cached in the index")
	}
	if len(vec) != catalog.VectorDim {
		t.Fatalf("cached tool vector dimension = %d, want %d (local fallback)", len(vec), catalog.VectorDim)
	}

	// Both fallback branches must leave an explicit log signal.
	logs := logBuf.String()
	if !strings.Contains(logs, "query vector") || !strings.Contains(logs, "falling back to local embedder") {
		t.Fatalf("query-vector dimension mismatch was not logged; logs:\n%s", logs)
	}
	if !strings.Contains(logs, "vector for chrome_devtools.take_snapshot") {
		t.Fatalf("per-tool dimension mismatch was not logged; logs:\n%s", logs)
	}
}
