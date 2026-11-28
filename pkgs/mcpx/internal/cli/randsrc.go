package cli

import (
	"io"

	"github.com/developing-today/code/pkgs/mcpx/internal/randsrc"
)

// RandSource is the source InstanceID mints from (#2640).
//
// The production value is randsrc.Crypto -- crypto/rand -- and must
// stay that: the instance id distinguishes this daemon from any other for the self-connection guard, so it has to stay unguessable. Tests swap in randsrc.Seeded so a run seed
// reproduces the ids a run minted; restore the old value when the
// test ends.
var RandSource io.Reader = randsrc.Crypto
