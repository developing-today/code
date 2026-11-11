//go:build !unix

package source

func isSymlinkLoop(error) bool { return false }
