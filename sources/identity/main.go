package main

import (
	"fmt"
	"os"

	"github.com/developing-today/code/src/identity/cmd"
)

func main() {
	configuration := cmd.LoadDefaultConfiguration()

	if err := cmd.RootCmd(configuration).Execute(); err != nil {
		fmt.Println(err)
		os.Exit(1)
	}
}
