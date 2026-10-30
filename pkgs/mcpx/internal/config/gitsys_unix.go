//go:build unix

package config

import "syscall"

// xOK is X_OK from <unistd.h>, which package syscall does not export.
const xOK = 0x1

// gitDevice is the device a path lives on, for the filesystem-boundary rule.
// A variable so tests can put a mount point where they cannot mount one.
var gitDevice = func(path string) (uint64, error) {
	var st syscall.Stat_t
	if err := syscall.Stat(path, &st); err != nil {
		return 0, err
	}
	// int32 on darwin, uint64 on linux; only equality matters.
	return uint64(st.Dev), nil
}

// accessX is access(path, X_OK), which is how git checks objects/ and refs/:
// present and searchable, not merely present.
func accessX(path string) bool { return syscall.Access(path, xOK) == nil }

// dirIdentity is a directory's (device, inode) pair: the identity two
// spellings of one directory share and two different directories never do,
// on either kind of volume. A variable, like gitDevice, so tests can put a
// case-insensitive volume where they cannot mount one.
var dirIdentity = func(path string) (uint64, uint64, bool) {
	var st syscall.Stat_t
	if err := syscall.Stat(path, &st); err != nil {
		return 0, 0, false
	}
	// Dev is int32 on darwin, uint64 on linux; Ino is uint64 on both. Only
	// equality matters.
	return uint64(st.Dev), uint64(st.Ino), true
}
