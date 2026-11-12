package cli

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/developing-today/code/pkgs/mcpx/internal/defaults"
)

// writeMarkdownReference implements the markdown form of CmdMan
// (`mcpx man --format markdown`): the generated reference tree of
// manual_md.go, written under dir, or printed to stdout as one
// document when dir is empty.
func (a *App) writeMarkdownReference(dir string) error {
	tree := ManualMarkdown(a.Version)
	paths := make([]string, 0, len(tree))
	for p := range tree {
		paths = append(paths, p)
	}
	sort.Strings(paths)
	if dir == "" {
		for _, p := range paths {
			fmt.Printf("<!-- %s -->\n\n%s", p, tree[p])
			if !strings.HasSuffix(tree[p], "\n") {
				fmt.Println()
			}
		}
		return nil
	}
	for _, p := range paths {
		full := filepath.Join(dir, p)
		if err := os.MkdirAll(filepath.Dir(full), defaults.PublicDirMode); err != nil {
			return err
		}
		if err := os.WriteFile(full, []byte(tree[p]), defaults.PublicMode); err != nil {
			return err
		}
	}
	fmt.Println(dir)
	return nil
}
