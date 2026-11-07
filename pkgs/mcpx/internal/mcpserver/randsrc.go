package mcpserver

import (
	"io"

	"github.com/developing-today/code/pkgs/mcpx/internal/randsrc"
)

// RandSource is the source newSessionID, newStateSigner and the requestState nonce mints from (#2640).
//
// The production value is randsrc.Crypto -- crypto/rand -- and must
// stay that: a session id is a bearer handle and the state signer key is secret material, so both have to stay unguessable and unreproducible in production. Tests swap in randsrc.Seeded so a run seed
// reproduces the ids a run minted; restore the old value when the
// test ends.
var RandSource io.Reader = randsrc.Crypto
