// Package seed is the lazy run-seed helper for mcpx's randomized
// tests (#1949 section 4, Approach C): the one channel by which the
// per-run seed reaches a test process is the MCPX_TEST_SEED env var,
// and this package is its only reader.
//
// Why lazy: a package that never calls this helper never reads the
// env var, so its test results stay cacheable by go test (a test
// that reads an env var is cached keyed on that var's value; the
// packages that do use randomness are exactly the ones that must
// re-run when the seed changes). There is no TestMain anywhere.
//
// Behaviour by situation (#1949 section 5):
//
//   - MCPX_TEST_SEED set (wrapper locally, the seed job in CI):
//     used as-is after validation. On input any string matching
//     [A-Za-z0-9._-]{1,128} is accepted, so a human can replay an
//     old id; only the wrapper and CI mint, and they mint UUIDv7.
//   - Unset outside CI (bare go test): a seed is minted once per
//     process and a warning is logged on first use — reproducible
//     (the id is printed on failure), but per-package, which is
//     why the wrapper (pkgs/mcpx/scripts/test.sh) is the entrypoint
//     that gives one seed per run.
//   - Unset in CI (CI=true or GITHUB_ACTIONS=true): fail closed.
//     CI must never mint inside a test process; the workflow's
//     seed job mints once per run and passes the id down.
//
// Allowlist note (#172): MCPX_TEST_SEED is a test-only variable —
// it is read only here, under internal/testsupport, and no setting
// declares it (the MCPX_TEST_* prefix is reserved for this class;
// no settings path may start with test.). This comment is the
// allowlist entry the env-inventory guard points at, in the same
// shape as MCPX_TEST_REQUIRE's in internal/testsupport/require.go.
package seed

import (
	"crypto/rand"
	"crypto/sha256"
	"encoding/binary"
	"fmt"
	"os"
	"regexp"
	"sync"
	"testing"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/testsupport/gen"
)

// EnvVar is the one channel: the run seed env var name.
const EnvVar = "MCPX_TEST_SEED"

// validID is the input contract (#1949 section 1): minting produces
// canonical UUIDv7, but any id of this shape replays.
var validID = regexp.MustCompile(`^[A-Za-z0-9._-]{1,128}$`)

// Valid reports whether id is an acceptable run-seed id on input.
func Valid(id string) bool { return validID.MatchString(id) }

// Mint returns a new canonical lowercase UUIDv7 (RFC 9562 section
// 5.7): 48-bit unix-millisecond timestamp, version 7, variant 10xx,
// and every remaining bit from crypto/rand — 74 random bits, more
// than google/uuid's NewV7 (which spends rand_a on sub-millisecond
// clock precision). No dependency; the wrapper's python3 one-liner
// and the CI mint step produce the same format.
func Mint() string {
	var b [16]byte
	ms := uint64(time.Now().UnixMilli())
	b[0], b[1], b[2] = byte(ms>>40), byte(ms>>32), byte(ms>>24)
	b[3], b[4], b[5] = byte(ms>>16), byte(ms>>8), byte(ms)
	if _, err := rand.Read(b[6:]); err != nil {
		// crypto/rand failing is unrecoverable for minting; a
		// time-only id would silently collide across runs.
		panic(fmt.Sprintf("seed: crypto/rand: %v", err))
	}
	b[6] = b[6]&0x0f | 0x70 // version 7
	b[8] = b[8]&0x3f | 0x80 // variant 10xx
	return fmt.Sprintf("%x-%x-%x-%x-%x", b[0:4], b[4:6], b[6:8], b[8:10], b[10:16])
}

// Shuffle derives the go test -shuffle value for a run seed
// (#1949 section 2): int64(big-endian(SHA-256(root || "shuffle")[:8])
// & MaxInt63), where root is gen.Root(seed). The wrapper script and
// the CI stress jobs implement the same formula in python3; the
// three implementations are pinned to each other by this package's
// tests (which restate the formula from the spec, not from gen).
func Shuffle(seed string) int64 {
	root := gen.Root(seed)
	h := sha256.Sum256(append(root[:], []byte("shuffle")...))
	return int64(binary.BigEndian.Uint64(h[:8]) & (1<<63 - 1))
}

// resolveFrom is the resolution policy over an env getter, split
// out so tests can drive every branch without the process-wide
// once. minted reports whether the id was minted here (bare go
// test), which is the case the caller must warn about.
func resolveFrom(get func(string) string) (id string, minted bool, err error) {
	if v := get(EnvVar); v != "" {
		if !Valid(v) {
			return "", false, fmt.Errorf("seed: %s=%q is not a valid seed id ([A-Za-z0-9._-]{1,128})", EnvVar, v)
		}
		return v, false, nil
	}
	if get("CI") == "true" || get("GITHUB_ACTIONS") == "true" {
		return "", false, fmt.Errorf("seed: %s is unset in CI; CI must never mint inside a test process — the workflow seed job (or the wrapper outside CI) provides it", EnvVar)
	}
	return Mint(), true, nil
}

var (
	once     sync.Once
	resolved string
	resErr   error
)

// Run returns this process's run seed, resolving MCPX_TEST_SEED
// lazily on first use (see the package comment for the policy). It
// fails the test on any resolution error: in CI that is the
// fail-closed rule, and an invalid id is never silently replaced.
func Run(t *testing.T) string {
	t.Helper()
	once.Do(func() {
		var minted bool
		resolved, minted, resErr = resolveFrom(os.Getenv)
		if resErr == nil && minted {
			fmt.Fprintf(os.Stderr, "seed: no %s; generated %s for this package only — use pkgs/mcpx/scripts/test.sh for one seed per run\n", EnvVar, resolved)
		}
	})
	if resErr != nil {
		t.Fatalf("%v", resErr)
	}
	return resolved
}

// Source returns the gen.Source stream for (this test,
// generator name@version) under the run seed — the lazy helper
// #1949 section 4 specifies. It registers a cleanup that, if the
// test failed, logs the seed, test and generator plus the exact
// one-command replay (#1949 section 6).
func Source(t *testing.T, name string, version int) gen.Source {
	t.Helper()
	id := Run(t)
	src := gen.ForTest(t, id, name, version)
	t.Cleanup(func() {
		if t.Failed() {
			t.Logf("seed: reproduce with: %s=%s pkgs/mcpx/scripts/test.sh -run '^%s$' <package> (seed %s, test %s, generator %s@%d)", EnvVar, id, t.Name(), id, t.Name(), name, version)
		}
	})
	return src
}
