package daemon

import (
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"
)

// Per-key daemon credentials (#13659 slices 2 and 3; design record
// docs/daemon-auth.md).
//
// A key is an opaque bearer secret: 32 random bytes, base64url, shown
// with the prefix mcpx_key_. The daemon keeps only the SHA-256 of each
// secret, in auth.json (0600) in its state directory (0700), so the
// store on disk cannot be replayed as credentials -- the same at-rest
// rule the public door applies to its own tokens. The secret itself is
// printed once, at creation, and never again.
//
// The store is consulted per request and re-read whenever the file
// changes, so a revocation takes effect on the next request without a
// restart. Ceilings are recorded on each key but not yet enforced:
// capability ceilings are slice 4.

// KeyPrefix marks an mcpx daemon key. It aids recognition in logs and
// lets the gate tell a daemon credential from an unrelated
// Authorization header; it is not part of the entropy claim.
const KeyPrefix = "mcpx_key_"

// KeyStoreFile is the key store's file name in the daemon state
// directory.
const KeyStoreFile = "auth.json"

// keySecretBytes is the random part of a key.
const keySecretBytes = 32

// keyIDLen is the length, in hex digits, of a key's public short ID.
const keyIDLen = 12

// Ceiling names for the starter ceilings the design declares. A key
// with no ceiling named gets CeilingDiscovery, never the full surface.
const (
	CeilingFull      = "full"
	CeilingDiscovery = "discovery"
	CeilingAdmin     = "admin"
)

// LegacyTokenLabel labels the virtual key transport.bearerToken
// resolves to while that setting is still honoured.
const LegacyTokenLabel = "legacy-transport-token"

// KeyRecord is one key as stored. It never holds the secret.
type KeyRecord struct {
	// ID is the public short ID, derived from the hash. It is safe to
	// log, list and pass to revoke.
	ID string `json:"id"`
	// Hash is the hex SHA-256 of the whole secret, prefix included.
	Hash          string     `json:"hash"`
	Label         string     `json:"label"`
	User          string     `json:"user"`
	Ceiling       string     `json:"ceiling"`
	CapabilityRef string     `json:"capability_ref,omitempty"`
	CreatedAt     time.Time  `json:"created_at"`
	LastUsedAt    *time.Time `json:"last_used_at,omitempty"`
	Revoked       bool       `json:"revoked,omitempty"`
	RevokedAt     *time.Time `json:"revoked_at,omitempty"`
}

// keyFile is auth.json on disk.
type keyFile struct {
	Version int         `json:"version"`
	Keys    []KeyRecord `json:"keys"`
}

// KeyIdentity is who an authenticated request is: the key it
// presented and the user that key belongs to. It is what audit
// stamping (slice 3) and ceiling enforcement (slice 4) read.
type KeyIdentity struct {
	KeyID         string
	User          string
	Label         string
	Ceiling       string
	CapabilityRef string
}

// HashKey returns the stored form of a secret: hex SHA-256.
func HashKey(secret string) string {
	sum := sha256.Sum256([]byte(secret))
	return hex.EncodeToString(sum[:])
}

// keyIDFromHash derives the public short ID from a key's hash.
func keyIDFromHash(hash string) string {
	if len(hash) < keyIDLen {
		return hash
	}
	return hash[:keyIDLen]
}

// NewKeySecret draws a fresh key secret from rnd (crypto/rand when nil).
func NewKeySecret(rnd io.Reader) (string, error) {
	if rnd == nil {
		rnd = rand.Reader
	}
	b := make([]byte, keySecretBytes)
	if _, err := io.ReadFull(rnd, b); err != nil {
		return "", fmt.Errorf("drawing a key: %w", err)
	}
	return KeyPrefix + base64.RawURLEncoding.EncodeToString(b), nil
}

// KeyStore is the hash-at-rest key store in one state directory.
type KeyStore struct {
	path string

	mu      sync.Mutex
	loaded  bool
	modTime time.Time
	size    int64
	byHash  map[string]KeyRecord
}

// OpenKeyStore returns the store for a state directory. Nothing is read
// until the first lookup; a missing file is an empty store.
func OpenKeyStore(stateDir string) *KeyStore {
	return &KeyStore{path: filepath.Join(stateDir, KeyStoreFile)}
}

// Path is where the store lives.
func (k *KeyStore) Path() string { return k.path }

// readFile reads auth.json; a missing file is an empty store.
func (k *KeyStore) readFile() (keyFile, error) {
	var f keyFile
	b, err := os.ReadFile(k.path)
	if errors.Is(err, os.ErrNotExist) {
		return keyFile{Version: 1}, nil
	}
	if err != nil {
		return f, err
	}
	if err := json.Unmarshal(b, &f); err != nil {
		return f, fmt.Errorf("%s: %w", k.path, err)
	}
	return f, nil
}

// writeFile replaces auth.json atomically with mode 0600.
func (k *KeyStore) writeFile(f keyFile) error {
	if f.Version == 0 {
		f.Version = 1
	}
	b, err := json.MarshalIndent(f, "", "  ")
	if err != nil {
		return err
	}
	dir := filepath.Dir(k.path)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return err
	}
	tmp, err := os.CreateTemp(dir, ".auth-*.json")
	if err != nil {
		return err
	}
	tmpName := tmp.Name()
	defer func() { _ = os.Remove(tmpName) }()
	if err := tmp.Chmod(0o600); err != nil {
		_ = tmp.Close()
		return err
	}
	if _, err := tmp.Write(append(b, '\n')); err != nil {
		_ = tmp.Close()
		return err
	}
	if err := tmp.Close(); err != nil {
		return err
	}
	if err := os.Rename(tmpName, k.path); err != nil {
		return err
	}
	k.mu.Lock()
	k.loaded = false
	k.mu.Unlock()
	return nil
}

// refresh reloads the index when the file changed since the last load.
// Callers hold k.mu.
func (k *KeyStore) refresh() error {
	st, err := os.Stat(k.path)
	switch {
	case errors.Is(err, os.ErrNotExist):
		k.byHash, k.loaded = map[string]KeyRecord{}, true
		k.modTime, k.size = time.Time{}, 0
		return nil
	case err != nil:
		return err
	}
	if k.loaded && st.ModTime().Equal(k.modTime) && st.Size() == k.size {
		return nil
	}
	f, err := k.readFile()
	if err != nil {
		return err
	}
	idx := make(map[string]KeyRecord, len(f.Keys))
	for _, r := range f.Keys {
		idx[r.Hash] = r
	}
	k.byHash, k.loaded = idx, true
	k.modTime, k.size = st.ModTime(), st.Size()
	return nil
}

// Lookup resolves a presented secret to its key. A revoked key, an
// unknown key and an unreadable store all answer false: the gate fails
// closed. The lookup is by hash, so no comparison is made against a
// stored secret and the store holds nothing a timing oracle could leak.
func (k *KeyStore) Lookup(secret string) (KeyRecord, bool) {
	if k == nil || !strings.HasPrefix(secret, KeyPrefix) {
		return KeyRecord{}, false
	}
	k.mu.Lock()
	defer k.mu.Unlock()
	if err := k.refresh(); err != nil {
		return KeyRecord{}, false
	}
	r, ok := k.byHash[HashKey(secret)]
	if !ok || r.Revoked {
		return KeyRecord{}, false
	}
	return r, true
}

// KeySpec is what creating a key needs.
type KeySpec struct {
	User          string
	Label         string
	Ceiling       string
	CapabilityRef string
}

// Create provisions a key and returns its secret, which is the only
// time the secret exists outside the caller's hands. An empty ceiling
// is the discovery ceiling, never the full surface.
func (k *KeyStore) Create(spec KeySpec, rnd io.Reader, now time.Time) (string, KeyRecord, error) {
	if strings.TrimSpace(spec.User) == "" {
		return "", KeyRecord{}, errors.New("a key needs a user")
	}
	if strings.TrimSpace(spec.Label) == "" {
		return "", KeyRecord{}, errors.New("a key needs a label")
	}
	if spec.Ceiling == "" {
		spec.Ceiling = CeilingDiscovery
	}
	secret, err := NewKeySecret(rnd)
	if err != nil {
		return "", KeyRecord{}, err
	}
	f, err := k.readFile()
	if err != nil {
		return "", KeyRecord{}, err
	}
	hash := HashKey(secret)
	rec := KeyRecord{
		ID:            keyIDFromHash(hash),
		Hash:          hash,
		Label:         spec.Label,
		User:          spec.User,
		Ceiling:       spec.Ceiling,
		CapabilityRef: spec.CapabilityRef,
		CreatedAt:     now.UTC(),
	}
	for _, r := range f.Keys {
		if r.ID == rec.ID {
			return "", KeyRecord{}, fmt.Errorf("key ID %s collides with an existing key; create again", rec.ID)
		}
	}
	f.Keys = append(f.Keys, rec)
	if err := k.writeFile(f); err != nil {
		return "", KeyRecord{}, err
	}
	return secret, rec, nil
}

// List returns every key record, revoked ones included, oldest first.
func (k *KeyStore) List() ([]KeyRecord, error) {
	f, err := k.readFile()
	if err != nil {
		return nil, err
	}
	out := append([]KeyRecord(nil), f.Keys...)
	sort.SliceStable(out, func(i, j int) bool { return out[i].CreatedAt.Before(out[j].CreatedAt) })
	return out, nil
}

// Revoke marks the key with this public ID revoked. Revoking is
// permanent: rotation is create-new then revoke-old.
func (k *KeyStore) Revoke(id string, now time.Time) error {
	f, err := k.readFile()
	if err != nil {
		return err
	}
	for i := range f.Keys {
		if f.Keys[i].ID != id {
			continue
		}
		if f.Keys[i].Revoked {
			return nil
		}
		t := now.UTC()
		f.Keys[i].Revoked, f.Keys[i].RevokedAt = true, &t
		return k.writeFile(f)
	}
	return fmt.Errorf("no key with ID %q", id)
}
