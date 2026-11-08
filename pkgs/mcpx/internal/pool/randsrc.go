package pool

import (
	"io"

	"github.com/developing-today/code/pkgs/mcpx/internal/randsrc"
)

// RandSource is the source newTraceID mints from (#2640).
//
// The production value is randsrc.Crypto -- crypto/rand -- and must
// stay that: a trace id is only a correlation handle, but production ids stay unguessable for the same reason. Tests swap in randsrc.Seeded so a run seed
// reproduces the ids a run minted; restore the old value when the
// test ends.
var RandSource io.Reader = randsrc.Crypto
