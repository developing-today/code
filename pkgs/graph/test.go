package main

import (
	"log"

	"github.com/dgraph-io/badger/v4"
)

func main() {
	opt := badger.DefaultOptions("/tmp/test")
	db, err := badger.Open(opt)
	if err != nil {
		log.Fatal(err)
	}
	defer db.Close()
}
