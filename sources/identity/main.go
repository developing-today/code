package main

import (
	"context"
	"os"

	"github.com/charmbracelet/log"
	"github.com/developing-today/code/src/identity/cmd"
)

func main() {
	ctx := context.Background()
	configuration := cmd.LoadDefaultConfiguration()

	if err := cmd.RootCmd(ctx, configuration).Execute(); err != nil {
		log.Error(err)
		os.Exit(1)
	}
}
