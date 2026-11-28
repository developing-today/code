//go:build !unix

package config

import "os"

// Without a device number there is no filesystem boundary to stop at, which
// is what GIT_DISCOVERY_ACROSS_FILESYSTEM=1 means anyway.
var gitDevice = func(string) (uint64, error) { return 0, nil }

func accessX(path string) bool { _, err := os.Stat(path); return err == nil }

// dirIdentity has no portable non-unix form: a Windows file index needs a
// handle API package syscall does not expose. Returning not-ok makes scope
// keys fall back to the canonical path alone, which is today's behaviour --
// over-isolation, never accidental sharing.
var dirIdentity = func(string) (uint64, uint64, bool) { return 0, 0, false }
