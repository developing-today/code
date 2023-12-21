package auth

import (
	"database/sql"
	"encoding/base64"
	"fmt"
	"os"
	"time"

	"github.com/nrednav/cuid2"

	"github.com/charmbracelet/log"
	"github.com/charmbracelet/ssh"
	_ "github.com/tursodatabase/libsql-client-go/libsql"
)

type Result struct {
	Name               string
	Roles              []string
	ID                 string
	CreatedAt          time.Time
	PublicKeyType      string
	PublicKeyString    string
	PublicKey          ssh.PublicKey
	PublicKeyCreatedAt time.Time
}

func CheckPublicKey(ctx ssh.Context, key ssh.PublicKey) (*Result, error) {
	host := os.Getenv("TURSO_HOST")
	if host == "" {
		log.Fatal("TURSO_HOST is not set")
	}
	authToken := os.Getenv("TURSO_AUTH_TOKEN")
	if authToken == "" {
		log.Fatal("TURSO_AUTH_TOKEN is not set")
	}
	db, err := sql.Open("libsql", fmt.Sprintf("libsql://%s?authToken=%s", host, authToken))
	if err != nil {
		return nil, fmt.Errorf("failed to open db %s: %w", host, err)
	}
	defer db.Close()

	publicKeyType := key.Type()
	publicKeyString := base64.StdEncoding.EncodeToString(key.Marshal())

	log.Info("Checking public key", "publicKeyType", publicKeyType, "publicKeyString", publicKeyString)

	stmt, err := db.Prepare("SELECT cu.charm_id, COALESCE(cu.name, ''), cu.created_at, pk.created_at FROM charm_user cu JOIN public_key pk ON pk.user_id = cu.id WHERE pk.public_key = ?")
	if err != nil {
		return nil, fmt.Errorf("failed to prepare query: %w", err)
	}
	defer stmt.Close()

	var res Result
	err = stmt.QueryRow(publicKeyType+" "+publicKeyString).Scan(&res.ID, &res.Name, &res.CreatedAt, &res.PublicKeyCreatedAt)
	if err != nil {
		if err == sql.ErrNoRows {
			return nil, fmt.Errorf("no user found with given public key")
		}
		return nil, fmt.Errorf("failed to execute query: %w", err)
	}

	res.Roles = []string{"admin"} // TODO: get roles from db
	res.PublicKey = key
	res.PublicKeyType = publicKeyType
	res.PublicKeyString = publicKeyString

	return &res, nil
}

func InsertPublicKey(user_id int64, key ssh.PublicKey) (int64, error) {
	host := os.Getenv("TURSO_HOST")
	if host == "" {
		log.Fatal("TURSO_HOST is not set")
	}
	authToken := os.Getenv("TURSO_AUTH_TOKEN")
	if authToken == "" {
		log.Fatal("TURSO_AUTH_TOKEN is not set")
	}
	db, err := sql.Open("libsql", fmt.Sprintf("libsql://%s?authToken=%s", host, authToken))
	if err != nil {
		return -1, fmt.Errorf("failed to open db %s: %w", host, err)
	}
	defer db.Close()

	publicKeyType := key.Type()
	publicKeyString := base64.StdEncoding.EncodeToString(key.Marshal())

	log.Info("Inserting public key", "publicKeyType", publicKeyType, "publicKeyString", publicKeyString)

	stmt, err := db.Prepare("INSERT INTO public_key (user_id, public_key) VALUES (?, ?)")
	if err != nil {
		return -1, fmt.Errorf("failed to prepare query: %w", err)
	}
	defer stmt.Close()

	result, err := stmt.Exec(user_id, publicKeyType+" "+publicKeyString)
	if err != nil {
		return -1, fmt.Errorf("failed to execute query: %w", err)
	}

	insertedId, err := result.LastInsertId()
	if err != nil {
		return -1, fmt.Errorf("failed to retrieve last insert id: %w", err)
	}

	return insertedId, nil
}

func InsertUser(ctx ssh.Context) (int64, error) {
	host := os.Getenv("TURSO_HOST")
	if host == "" {
		log.Fatal("TURSO_HOST is not set")
	}
	authToken := os.Getenv("TURSO_AUTH_TOKEN")
	if authToken == "" {
		log.Fatal("TURSO_AUTH_TOKEN is not set")
	}
	db, err := sql.Open("libsql", fmt.Sprintf("libsql://%s?authToken=%s", host, authToken))
	if err != nil {
		return -1, fmt.Errorf("failed to open db %s: %w", host, err)
	}
	defer db.Close()

	stmt, err := db.Prepare("INSERT INTO charm_user (charm_id, bio) VALUES (?, ?)")
	if err != nil {
		return -1, fmt.Errorf("failed to prepare insert statement: %w", err)
	}
	defer stmt.Close()

	id := cuid2.Generate()

	result, err := stmt.Exec(id, ctx.RemoteAddr().Network()+":"+ctx.RemoteAddr().String())
	if err != nil {
		return -1, fmt.Errorf("failed to execute insert: %w", err)
	}

	insertedId, err := result.LastInsertId()
	if err != nil {
		return -1, fmt.Errorf("failed to retrieve last insert id: %w", err)
	}

	log.Info("User inserted", "charm_id", id, "insertedId", insertedId)

	return insertedId, nil
}
