package main

import (
	"fmt"
	"os"
	"path"

	"github.com/charmbracelet/charm/kv"
	"github.com/charmbracelet/log"
	badger "github.com/dgraph-io/badger/v4"
)

func handle(err error) {
	if err != nil {
		log.Fatal(err)
	}
}

func main() {
	log.Info("Hello")

	e, err := os.Executable()
	if err != nil {
		fmt.Println(err)
		return
	}
	fmt.Println(path.Dir(e))

	// It will be created if it doesn't exist.
	opts := badger.DefaultOptions("badger")
	db, err := badger.Open(opts)
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
		// log.Infof("The answer is: %s\n", val)

		// Copying or parsing val is valid.
		// valCopy = append([]byte{}, val...)

		// Assigning val slice to another variable is NOT OK.
		// valNot = val // Do not do this.
		// return nil
		// })
		// handle(err)

		// DO NOT access val here. It is the most common cause of bugs.
		// log.Infof("NEVER do this. %s\n", valNot)

		// You must copy it to use it outside item.Value(...).
		// log.Infof("The answer is: %s\n", valCopy)

		// Alternatively, you could also use item.ValueCopy().
		valCopy, err = item.ValueCopy(nil)
		handle(err)
		log.Info("", "answer", string(valCopy))

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
		log.Info("My fave food is:")
		log.Info("", "food", string(food))
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
		log.Info("Is your fave food:")
		log.Info("", "food", string(food))
	}
}
