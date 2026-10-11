//go:build !unix

package config

import "os"

// fileInode has no portable form off unix. Size, mtime and mode still tell
// a rewritten file apart, and the real path tells a re-pointed one apart.
func fileInode(fi os.FileInfo) string { return "" }
