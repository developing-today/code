package main

import (
	"os"

	"github.com/charmbracelet/log"
	"github.com/developing-today/code/src/identity/cmd/root"
)

func main() {
	if err := root.DefaultRootCmd().Execute(); err != nil {
		log.Error(err)
		os.Exit(1)
	}
}
