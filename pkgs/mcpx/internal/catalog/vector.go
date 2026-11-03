package catalog

import (
	"database/sql"
	"encoding/binary"
	"fmt"
	"log"
	"math"
	"strings"
	"sync"
	"unicode"

	_ "modernc.org/sqlite"
)

const VectorDim = 384

// DimensionError reports a vector whose length does not match the index
// dimension (VectorDim). Writes carrying such a vector are rejected, so a
// mismatched embedding can never enter the index silently and then vanish
// on reload (loadFromDB can only load VectorDim rows).
type DimensionError struct {
	Expected int
	Actual   int
}

func (e *DimensionError) Error() string {
	return fmt.Sprintf("catalog: vector dimension mismatch: expected %d dimensions, got %d", e.Expected, e.Actual)
}

// checkDim returns a *DimensionError when vec is not exactly VectorDim long.
func checkDim(vec Vector) error {
	if len(vec) != VectorDim {
		return &DimensionError{Expected: VectorDim, Actual: len(vec)}
	}
	return nil
}

// Vector is a normalized dense embedding vector of float32s.
type Vector []float32

// CosineSimilarity computes the dot product of two normalized vectors.
func (v Vector) CosineSimilarity(other Vector) float32 {
	if len(v) != len(other) || len(v) == 0 {
		return 0
	}
	var dot float32
	for i := 0; i < len(v); i++ {
		dot += v[i] * other[i]
	}
	return dot
}

// SemanticClusters contains intent synonyms that bridge natural language queries to MCP tool domains.
var semanticClusters = map[string][]string{
	"browser": {"inspect", "web", "page", "click", "snapshot", "dom", "element", "screenshot", "navigate", "url", "button", "html", "css", "scrape", "crawl"},
	"vcs":     {"git", "commit", "branch", "diff", "checkout", "push", "pull", "repo", "repository", "status", "merge", "log"},
	"file":    {"read", "write", "path", "file", "directory", "folder", "list", "delete", "edit", "create", "content", "filesystem"},
	"exec":    {"run", "execute", "shell", "bash", "command", "terminal", "process", "cmd", "sh"},
	"data":    {"sql", "query", "database", "table", "select", "insert", "schema", "column", "row"},
}

// Embed generates a 384-dimensional normalized dense embedding vector from text.
func Embed(text string) Vector {
	vec := make([]float32, VectorDim)
	tokens := tokenize(text)
	if len(tokens) == 0 {
		return vec
	}

	// 1. Project tokens and character n-grams into the vector
	for _, tok := range tokens {
		projectToken(vec, tok, 1.0)
		// Character trigrams
		if len(tok) >= 3 {
			runes := []rune(tok)
			for i := 0; i <= len(runes)-3; i++ {
				ngram := string(runes[i : i+3])
				projectToken(vec, ngram, 0.4)
			}
		}
		// Semantic cluster expansion
		for clusterKey, clusterWords := range semanticClusters {
			for _, w := range clusterWords {
				if tok == w {
					projectToken(vec, clusterKey, 0.7)
					break
				}
			}
		}
	}

	// 2. Normalize to unit length (L2 norm)
	var normSq float32
	for _, val := range vec {
		normSq += val * val
	}
	if normSq > 0 {
		invNorm := float32(1.0 / math.Sqrt(float64(normSq)))
		for i := range vec {
			vec[i] *= invNorm
		}
	}
	return vec
}

func tokenize(s string) []string {
	var words []string
	var b strings.Builder
	for _, r := range s {
		if unicode.IsLetter(r) || unicode.IsDigit(r) {
			b.WriteRune(unicode.ToLower(r))
		} else {
			if b.Len() > 0 {
				words = append(words, b.String())
				b.Reset()
			}
		}
	}
	if b.Len() > 0 {
		words = append(words, b.String())
	}
	return words
}

// projectToken hashes token deterministically into VectorDim indices with pseudo-random weights.
func projectToken(vec []float32, tok string, weight float32) {
	h := fnv32(tok)
	// Distribute across 4 pseudo-random dimension buckets
	for i := 0; i < 4; i++ {
		dim := int((h + uint32(i*104729)) % VectorDim)
		sign := float32(1.0)
		if (h>>(i*7))&1 == 1 {
			sign = -1.0
		}
		vec[dim] += sign * weight
	}
}

func fnv32(s string) uint32 {
	var h uint32 = 2166136261
	for i := 0; i < len(s); i++ {
		h ^= uint32(s[i])
		h *= 16777619
	}
	return h
}

// VectorIndex holds tool embeddings in memory and optionally persists them to SQLite.
type VectorIndex struct {
	mu      sync.RWMutex
	vectors map[string]Vector
	db      *sql.DB
	// droppedOnLoad counts persisted rows loadFromDB skipped because their
	// blob was not a VectorDim vector.
	droppedOnLoad int
}

// DroppedOnLoad reports how many persisted rows loadFromDB skipped because
// they were not VectorDim vectors (rows written before the dimension
// contract, or by another writer). Zero for an in-memory-only index.
func (idx *VectorIndex) DroppedOnLoad() int {
	return idx.droppedOnLoad
}

// NewVectorIndex creates a vector index, optionally backed by SQLite dbPath.
func NewVectorIndex(dbPath string) (*VectorIndex, error) {
	idx := &VectorIndex{
		vectors: make(map[string]Vector),
	}
	if dbPath != "" {
		db, err := sql.Open("sqlite", dbPath)
		if err != nil {
			return nil, err
		}
		_, err = db.Exec(`
			CREATE TABLE IF NOT EXISTS tool_embeddings (
				key TEXT PRIMARY KEY,
				vector BLOB NOT NULL
			)
		`)
		if err != nil {
			_ = db.Close()
			return nil, err
		}
		idx.db = db
		idx.loadFromDB()
	}
	return idx, nil
}

func (idx *VectorIndex) loadFromDB() {
	if idx.db == nil {
		return
	}
	rows, err := idx.db.Query("SELECT key, vector FROM tool_embeddings")
	if err != nil {
		log.Printf("catalog: vector index: query persisted vectors: %v", err)
		return
	}
	defer rows.Close()

	for rows.Next() {
		var key string
		var blob []byte
		if err := rows.Scan(&key, &blob); err != nil {
			idx.droppedOnLoad++
			continue
		}
		if len(blob) != VectorDim*4 {
			// Not silently droppable: count and report below. Such rows
			// predate the dimension contract or came from another writer.
			idx.droppedOnLoad++
			continue
		}
		vec := make(Vector, VectorDim)
		for i := 0; i < VectorDim; i++ {
			bits := binary.LittleEndian.Uint32(blob[i*4 : (i+1)*4])
			vec[i] = math.Float32frombits(bits)
		}
		idx.vectors[key] = vec
	}
	if err := rows.Err(); err != nil {
		log.Printf("catalog: vector index: read persisted vectors: %v", err)
	}
	if idx.droppedOnLoad > 0 {
		log.Printf("catalog: vector index: skipped %d persisted vector row(s) whose dimension is not %d; they were not loaded", idx.droppedOnLoad, VectorDim)
	}
}

// IndexTool computes and stores an embedding for a tool.
func (idx *VectorIndex) IndexTool(key, text string) Vector {
	vec := Embed(text)
	idx.mu.Lock()
	idx.vectors[key] = vec
	idx.mu.Unlock()

	if idx.db != nil {
		blob := make([]byte, VectorDim*4)
		for i, v := range vec {
			binary.LittleEndian.PutUint32(blob[i*4:(i+1)*4], math.Float32bits(v))
		}
		_, _ = idx.db.Exec(`
			INSERT OR REPLACE INTO tool_embeddings (key, vector)
			VALUES (?, ?)
		`, key, blob)
	}
	return vec
}

// Put stores an explicit vector for a key in memory and on disk. A vector
// whose length is not VectorDim is rejected with a *DimensionError: storing
// it would persist a row loadFromDB cannot load back.
func (idx *VectorIndex) Put(key string, vec Vector) error {
	if err := checkDim(vec); err != nil {
		return err
	}
	idx.mu.Lock()
	idx.vectors[key] = vec
	idx.mu.Unlock()

	if idx.db != nil {
		blob := make([]byte, len(vec)*4)
		for i, v := range vec {
			binary.LittleEndian.PutUint32(blob[i*4:(i+1)*4], math.Float32bits(v))
		}
		_, err := idx.db.Exec(`
			INSERT OR REPLACE INTO tool_embeddings (key, vector)
			VALUES (?, ?)
		`, key, blob)
		return err
	}
	return nil
}

// Get retrieves the vector for a key if indexed.
func (idx *VectorIndex) Get(key string) (Vector, bool) {
	idx.mu.RLock()
	defer idx.mu.RUnlock()
	v, ok := idx.vectors[key]
	return v, ok
}

// Close closes the underlying SQLite database if open.
func (idx *VectorIndex) Close() error {
	idx.mu.Lock()
	defer idx.mu.Unlock()
	if idx.db != nil {
		return idx.db.Close()
	}
	return nil
}
