// Package randsrc is mcpx's injectable randomness seam (#2640).
//
// Production id minting reads from crypto/rand, and must keep doing
// so: artifact handles, task handles and session ids are access
// control, and a guessable id is a broken one. But a hard-wired
// crypto/rand means no test seed (#1949) can ever reproduce an id a
// run minted, so seeded inputs cannot compose with execution.
// Packages that mint ids therefore read from a package-level source
// variable whose production value is Crypto (this package), and a
// test swaps in Seeded to make the run reproducible.
//
// Seeded is a test source only. Its output is a pure function of the
// seed, so anyone who knows the seed knows every byte it will ever
// produce: never wire it into a production path, and never derive
// secret material from it (internal/secrets does not use this
// package at all, on purpose).
package randsrc

import (
	"crypto/rand"
	"crypto/sha256"
	"encoding/binary"
	"io"
)

// Source is a source of random bytes: the part of io.Reader the
// minting call sites use. crypto/rand.Reader is one; Seeded returns
// another.
type Source = io.Reader

// Crypto is the production source: crypto/rand itself.
var Crypto Source = rand.Reader

// Seeded returns a deterministic Source expanding an opaque seed.
//
// The seed is deliberately opaque bytes: #1949 decides the run
// seed's format, and this expansion must not hard-code one, so any
// seed string or byte string drops in. The label separates streams
// that share a seed -- one per minting package, say -- so two
// packages seeded alike do not mint the same ids.
//
// Expansion is SHA-256 in counter mode:
//
//	block(i) = SHA-256(seed || 0x00 || label || 0x00 || uint64be(i))
//
// and the stream is block(0) || block(1) || ... . Reads of any size
// slice the same stream: reading 8 then 8 yields the bytes reading
// 16 yields.
func Seeded(seed []byte, label string) Source {
	return &seeded{seed: append([]byte(nil), seed...), label: label}
}

type seeded struct {
	seed    []byte
	label   string
	counter uint64
	block   [sha256.Size]byte
	off     int
	filled  bool
}

func (s *seeded) Read(p []byte) (int, error) {
	n := 0
	for n < len(p) {
		if !s.filled || s.off == len(s.block) {
			h := sha256.New()
			h.Write(s.seed)
			h.Write([]byte{0})
			h.Write([]byte(s.label))
			h.Write([]byte{0})
			var c [8]byte
			binary.BigEndian.PutUint64(c[:], s.counter)
			h.Write(c[:])
			copy(s.block[:], h.Sum(nil))
			s.counter++
			s.off = 0
			s.filled = true
		}
		m := copy(p[n:], s.block[s.off:])
		s.off += m
		n += m
	}
	return n, nil
}
