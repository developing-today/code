package main

import (
	"os"

	"github.com/charmbracelet/log"
	"github.com/developing-today/code/src/identity/cmd"
)

func main() {
	if err := cmd.DefaultRootCmd().Execute(); err != nil {
		log.Error(err)
		os.Exit(1)
	}
}
