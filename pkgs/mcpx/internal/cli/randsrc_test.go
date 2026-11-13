package cli

import (
	"crypto/rand"
	"testing"
)

// The production default must stay crypto/rand (#2640): the
// instance id distinguishes this daemon from any other for the
// self-connection guard.
func TestRandSourceDefaultIsCryptoRand(t *testing.T) {
	if RandSource != rand.Reader {
		t.Fatal("production default RandSource is not crypto/rand")
	}
}
