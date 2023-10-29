package main

import (
	"fmt"
	"log"

	"github.com/charmbracelet/charm/kv"
	badger "github.com/dgraph-io/badger/v4"
)
func handle(err error) {
  if err != nil {
    log.Fatal(err)
  }
}

func main() {
	fmt.Println("Hello flake")

	// Open the Badger database located in the /tmp/badger directory.
  // It will be created if it doesn't exist.
  db, err := badger.Open(badger.DefaultOptions("badger"))
  handle(err)
  defer db.Close()

	err = db.Update(func(txn *badger.Txn) error {
		err := txn.Set([]byte("answer"), []byte("42"))
		return err
	})

	err = db.View(func(txn *badger.Txn) error {
		item, err := txn.Get([]byte("answer"))
		handle(err)

		var valCopy []byte
		// err = item.Value(func(val []byte) error {
			// This func with val would only be called if item.Value encounters no error.

			// Accessing val here is valid.
			// fmt.Printf("The answer is: %s\n", val)

			// Copying or parsing val is valid.
			// valCopy = append([]byte{}, val...)

			// Assigning val slice to another variable is NOT OK.
			// valNot = val // Do not do this.
		 // return nil
		// })
		// handle(err)

		// DO NOT access val here. It is the most common cause of bugs.
		// fmt.Printf("NEVER do this. %s\n", valNot)

		// You must copy it to use it outside item.Value(...).
		// fmt.Printf("The answer is: %s\n", valCopy)

		// Alternatively, you could also use item.ValueCopy().
		valCopy, err = item.ValueCopy(nil)
		handle(err)
		fmt.Printf("The answer is: %s\n", valCopy)

		return nil
	})

  cdb, err := kv.OpenWithDefaults("my-cute-db")
  if err != nil {
      log.Fatal(err)
  }
  defer cdb.Close()

  if food, err := cdb.Get([]byte("fave-food")); err != nil {
      log.Fatal(err)
  } else {
      fmt.Printf("My fave food is %s.\n", food)
  }

  if err := cdb.Sync(); err != nil {
      log.Fatal(err)
  }

  if err := cdb.Set([]byte("fave-food"), []byte("gherkin")); err != nil {
      log.Fatal(err)
  }

  if food, err := cdb.Get([]byte("fave-food")); err != nil {
    log.Fatal(err)
  } else {
    fmt.Printf("Is your fave food %s?\n", food)
  }
}
