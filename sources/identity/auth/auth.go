package auth

import (
	"crypto/ed25519"
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

func InsertPrivateKey(pk ed25519.PrivateKey) (int64, error) {
	log.Info("Inserting private key", "privateKeyType", pk.Public().(ed25519.PublicKey), "privateKeyString", base64.StdEncoding.EncodeToString(pk))
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

	publicKey := pk.Public().(ed25519.PublicKey)
	privateKeyString := base64.StdEncoding.EncodeToString(pk)
	publicKeyType := "ssh-ed25519"
	publicKeyString := base64.StdEncoding.EncodeToString(publicKey)

	log.Info("Inserting private key", "privateKeyType", publicKey, "privateKeyString", privateKeyString, "publicKeyType", publicKeyType, "publicKeyString", publicKeyString)

	stmt, err := db.Prepare("INSERT INTO private_key (type, private_key, public_key) VALUES (?, ?, ?)")
	if err != nil {
		return -1, fmt.Errorf("failed to prepare query: %w", err)
	}
	defer stmt.Close()

	log.Info("Inserting private key", "privateKeyType", publicKey, "privateKeyString", privateKeyString, "publicKeyType", publicKeyType, "publicKeyString", publicKeyString)

	result, err := stmt.Exec(publicKeyType, privateKeyString, publicKeyString)

	if err != nil {
		return -1, fmt.Errorf("failed to execute query: %w", err)
	}

	insertedId, err := result.LastInsertId()
	if err != nil {
		return -1, fmt.Errorf("failed to retrieve last insert id: %w", err)
	}
	rowsAffected, err := result.RowsAffected()
	if err != nil {
		return -1, fmt.Errorf("failed to retrieve rows affected: %w", err)
	}
	log.Info("Inserted private key", "insertedId", insertedId, "rowsAffected", rowsAffected)

	return insertedId, nil
}

func UpdatePrivateKey(id int64, charm_id *string, connection_id *string) (int64, error) {
	log.Info("Updating private key", "id", id, "charm_id", *charm_id, "connection_id", *connection_id)
	var affected int64
	if id == 0 || (charm_id == nil && connection_id == nil) {
		return 0, fmt.Errorf("no fields to update")
	}
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
		return 0, fmt.Errorf("failed to open db %s: %w", host, err)
	}
	defer db.Close()

	stmtBase := "UPDATE private_key SET "
	if charm_id != nil {
		stmtBase += "public_key = ? "
	}
	if connection_id != nil {
		if charm_id != nil {
			stmtBase += ", "
		}
		stmtBase += "connection_id = ? "
	}
	stmtBase += "WHERE rowid = ?"

	stmt, err := db.Prepare(stmtBase)
	if err != nil {
		return 0, fmt.Errorf("failed to prepare query: %w", err)
	}
	defer stmt.Close()

	if charm_id != nil && connection_id != nil {
		if charm_id != nil && *charm_id == "" {
			charm_id = nil
		}
		if connection_id != nil && *connection_id == "" {
			connection_id = nil
		}
		result, err := stmt.Exec(charm_id, connection_id, id)
		if err != nil {
			return 0, fmt.Errorf("failed to execute query: %w", err)
		}
		affected, err = result.RowsAffected()
		if err != nil {
			return 0, fmt.Errorf("failed to retrieve rows affected: %w", err)
		}
		rowid, err := result.LastInsertId()
		if err != nil {
			return 0, fmt.Errorf("failed to retrieve last insert id: %w", err)
		}
		log.Info("Updating private key", "id", id, "charm_id", *charm_id, "connection_id", *connection_id, "rowsAffected", affected, "rowid", rowid)
	} else if charm_id != nil {
		if charm_id != nil && *charm_id == "" {
			charm_id = nil
		}
		result, err := stmt.Exec(charm_id, id)
		if err != nil {
			return 0, fmt.Errorf("failed to execute query: %w", err)
		}
		affected, err = result.RowsAffected()
		if err != nil {
			return 0, fmt.Errorf("failed to retrieve rows affected: %w", err)
		}
		rowid, err := result.LastInsertId()
		if err != nil {
			return 0, fmt.Errorf("failed to retrieve last insert id: %w", err)
		}
		log.Info("Updating private key", "id", id, "charm_id", *charm_id, "connection_id", *connection_id, "rowsAffected", affected, "rowid", rowid)
	} else if connection_id != nil {
		if connection_id != nil && *connection_id == "" {
			connection_id = nil
		}
		result, err := stmt.Exec(connection_id, id)
		if err != nil {
			return 0, fmt.Errorf("failed to execute query: %w", err)
		}
		affected, err = result.RowsAffected()
		if err != nil {
			return 0, fmt.Errorf("failed to retrieve rows affected: %w", err)
		}
		rowid, err := result.LastInsertId()
		if err != nil {
			return 0, fmt.Errorf("failed to retrieve last insert id: %w", err)
		}
		log.Info("Updated private key", "id", id, "charm_id", *charm_id, "connection_id", *connection_id, "rowsAffected", affected, "rowid", rowid)
	}
	if err != nil {
		return 0, fmt.Errorf("failed to execute query: %w", err)
	}
	log.Info("Updated private key", "id", id, "charm_id", *charm_id, "connection_id", *connection_id)

	return affected, nil
}

func CheckPublicKey(ctx ssh.Context, key ssh.PublicKey) (*Result, error) {
	log.Info("Checking public key", "name", ctx.User(), "remoteAddr", ctx.RemoteAddr().Network()+":"+ctx.RemoteAddr().String(), "localAddr", ctx.LocalAddr().Network()+":"+ctx.LocalAddr().String())
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

	log.Info("Got public key", "publicKeyString", publicKeyString, "res", res)

	return &res, nil
}

func InsertPublicKey(user_id int64, key ssh.PublicKey) (int64, error) {
	log.Info("Inserting public key", "user_id", user_id)
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

	affect, err := result.RowsAffected()

	if err != nil {
		return -1, fmt.Errorf("failed to retrieve rows affected: %w", err)
	}

	log.Info("Inserted public key", "affect", affect)

	if affect == 0 {
		return -1, fmt.Errorf("failed to insert public key")
	}

	insertedId, err := result.LastInsertId()
	if err != nil {
		return -1, fmt.Errorf("failed to retrieve last insert id: %w", err)
	}
	log.Info("Inserted public key", "insertedId", insertedId, "affect", affect)

	return insertedId, nil
}

func InsertUser(ctx ssh.Context) (int64, error) {
	log.Info("Inserting user", "name", ctx.User(), "remoteAddr", ctx.RemoteAddr().Network()+":"+ctx.RemoteAddr().String(), "localAddr", ctx.LocalAddr().Network()+":"+ctx.LocalAddr().String())
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

	affect, err := result.RowsAffected()
	if err != nil {
		return -1, fmt.Errorf("failed to retrieve rows affected: %w", err)
	}
	log.Info("User inserted", "charm_id", id, "affect", affect)

	if affect == 0 {
		return -1, fmt.Errorf("failed to insert user")
	}

	insertedId, err := result.LastInsertId()
	if err != nil {
		return -1, fmt.Errorf("failed to retrieve last insert id: %w", err)
	}

	log.Info("User inserted", "charm_id", id, "insertedId", insertedId, "affect", affect)

	log.Info("User inserted", "charm_id", id, "insertedId", insertedId)

	return insertedId, nil
}

func InsertHashPublicKey(hash string, hash_type string, key ssh.PublicKey) (int64, error) {
	log.Info("Inserting hash public key", "hash", hash, "hash_type", hash_type)
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

	log.Info("Inserting hash public key", "publicKeyType", publicKeyType, "publicKeyString", publicKeyString)

	stmt, err := db.Prepare("INSERT INTO hash_public_key (hash, hash_type, public_key) VALUES (?, ?, ?)")
	if err != nil {
		return -1, fmt.Errorf("failed to prepare query: %w", err)
	}
	defer stmt.Close()

	result, err := stmt.Exec(hash, hash_type, publicKeyType+" "+publicKeyString)
	if err != nil {
		return -1, fmt.Errorf("failed to execute query: %w", err)
	}

	affect, err := result.RowsAffected()
	if err != nil {
		return -1, fmt.Errorf("failed to retrieve rows affected: %w", err)
	}
	log.Info("Inserted hash public key", "affect", affect)
	if affect == 0 {
		return -1, fmt.Errorf("failed to insert hash public key")
	}

	insertedId, err := result.LastInsertId()
	if err != nil {
		return -1, fmt.Errorf("failed to retrieve last insert id: %w", err)
	}
	log.Info("Inserted hash public key", "insertedId", insertedId, "affect", affect)

	return insertedId, nil
}

func GetPublicKeyFromHash(hash string, hash_type string) (string, error) {
	log.Info("Getting hash public key", "hash", hash, "hash_type", hash_type)
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
		return "", fmt.Errorf("failed to open db %s: %w", host, err)
	}
	defer db.Close()

	stmt, err := db.Prepare("SELECT public_key FROM hash_public_key WHERE hash = ? AND hash_type like ?")
	if err != nil {
		return "", fmt.Errorf("failed to prepare query: %w", err)
	}
	defer stmt.Close()

	var publicKeyString string
	err = stmt.QueryRow(hash, hash_type).Scan(&publicKeyString)
	if err != nil {
		if err == sql.ErrNoRows {
			return "", fmt.Errorf("no user found with given public key")
		}
		return "", fmt.Errorf("failed to execute query: %w", err)
	}

	log.Info("Got public key", "publicKeyString", publicKeyString)

	return publicKeyString, nil
}

func InsertTextPublicKey(text string, text_type string, key ssh.PublicKey) (int64, error) {
	log.Info("Inserting text public key", "text", text, "text_type", text_type)
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

	log.Info("Inserting text public key", "publicKeyType", publicKeyType, "publicKeyString", publicKeyString)

	stmt, err := db.Prepare("INSERT INTO text_public_key (text, text_type, public_key) VALUES (?, ?, ?)")
	if err != nil {
		return -1, fmt.Errorf("failed to prepare query: %w", err)
	}
	defer stmt.Close()

	result, err := stmt.Exec(text, text_type, publicKeyType+" "+publicKeyString)
	if err != nil {
		return -1, fmt.Errorf("failed to execute query: %w", err)
	}

	affect, err := result.RowsAffected()
	if err != nil {
		return -1, fmt.Errorf("failed to retrieve rows affected: %w", err)
	}
	log.Info("Inserted text public key", "affect", affect)
	if affect == 0 {
		return -1, fmt.Errorf("failed to insert text public key")
	}

	insertedId, err := result.LastInsertId()
	if err != nil {
		return -1, fmt.Errorf("failed to retrieve last insert id: %w", err)
	}

	log.Info("Inserted text public key", "insertedId", insertedId, "affect", affect)

	return insertedId, nil
}

func GetPublicKeyFromText(text string, text_type string) (string, error) {
	log.Info("Getting text public key", "text", text, "text_type", text_type)
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
		return "", fmt.Errorf("failed to open db %s: %w", host, err)
	}
	defer db.Close()

	stmt, err := db.Prepare("SELECT public_key FROM text_public_key WHERE text = ? AND text_type like ?")
	if err != nil {
		return "", fmt.Errorf("failed to prepare query: %w", err)
	}
	defer stmt.Close()

	var publicKeyString string
	err = stmt.QueryRow(text, text_type).Scan(&publicKeyString)
	if err != nil {
		if err == sql.ErrNoRows {
			return "", fmt.Errorf("no user found with given public key")
		}
		return "", fmt.Errorf("failed to execute query: %w", err)
	}
	log.Info("Got public key", "publicKeyString", publicKeyString)

	return publicKeyString, nil
}

func GetPublicKey(text string, text_type *string) (string, error) {
	log.Info("Getting hash/text public key", "text", text, "text_type", text_type)
	if text_type == nil {
		text_type = new(string)
		*text_type = "%"
	}
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
		return "", fmt.Errorf("failed to open db %s: %w", host, err)
	}
	defer db.Close()

	stmt, err := db.Prepare("SELECT public_key FROM HASH_public_key WHERE text = ? AND text_type like ? UNION SELECT public_key FROM text_public_key WHERE text = ? AND text_type like ?")
	if err != nil {
		return "", fmt.Errorf("failed to prepare query: %w", err)
	}
	defer stmt.Close()

	var publicKeyString string
	err = stmt.QueryRow(text, text_type, text, text_type).Scan(&publicKeyString)
	if err != nil {
		if err == sql.ErrNoRows {
			return "", fmt.Errorf("no user found with given public key")
		}
		return "", fmt.Errorf("failed to execute query: %w", err)
	}
	log.Info("Got public key", "publicKeyString", publicKeyString)

	return publicKeyString, nil
}

func UpdateHashPublicKey(id int64, charm_id *string, connection_id *string) (int64, error) {
	log.Info("Updating hash public key", "id", id, "charm_id", *charm_id, "connection_id", *connection_id)
	if id == 0 || (charm_id == nil && connection_id == nil) {
		return 0, fmt.Errorf("no fields to update")
	}
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
		return 0, fmt.Errorf("failed to open db %s: %w", host, err)
	}
	defer db.Close()

	stmtBase := "UPDATE hash_public_key SET "
	if charm_id != nil {
		stmtBase += "charm_id = ? "
	}
	if connection_id != nil {
		if charm_id != nil {
			stmtBase += ", "
		}
		stmtBase += "connection_id = ? "
	}
	stmtBase += "WHERE rowid = ?"

	stmt, err := db.Prepare(stmtBase)
	if err != nil {
		return 0, fmt.Errorf("failed to prepare query: %w", err)
	}
	defer stmt.Close()

	var affected int64
	if charm_id != nil && connection_id != nil {
		if charm_id != nil && *charm_id == "" {
			charm_id = nil
		}
		if connection_id != nil && *connection_id == "" {
			connection_id = nil
		}
		result, err := stmt.Exec(charm_id, connection_id, id)

		if err != nil {
			return 0, fmt.Errorf("failed to execute query: %w", err)
		}

		affected, err = result.RowsAffected()

		if err != nil {
			return 0, fmt.Errorf("failed to retrieve rows affected: %w", err)
		}

		if affected == 0 {
			return 0, fmt.Errorf("failed to update hash public key")
		}

		rowid, err := result.LastInsertId()

		if err != nil {
			return 0, fmt.Errorf("failed to retrieve last insert id: %w", err)
		}

		log.Info("Updating hash public key", "id", id, "charm_id", *charm_id, "connection_id", *connection_id, "affected", affected, "rowid", rowid)
	} else if charm_id != nil {
		if charm_id != nil && *charm_id == "" {
			charm_id = nil
		}
		result, err := stmt.Exec(charm_id, id)

		if err != nil {
			return 0, fmt.Errorf("failed to execute query: %w", err)
		}

		affected, err = result.RowsAffected()

		if err != nil {
			return 0, fmt.Errorf("failed to retrieve rows affected: %w", err)
		}

		if affected == 0 {
			return 0, fmt.Errorf("failed to update hash public key")
		}

		rowid, err := result.LastInsertId()

		if err != nil {

			return 0, fmt.Errorf("failed to retrieve last insert id: %w", err)
		}

		log.Info("Updating hash public key", "id", id, "charm_id", *charm_id, "connection_id", *connection_id, "affected", affected, "rowid", rowid)
	} else if connection_id != nil {
		if connection_id != nil && *connection_id == "" {
			connection_id = nil
		}
		result, err := stmt.Exec(connection_id, id)
		if err != nil {
			return 0, fmt.Errorf("failed to execute query: %w", err)
		}

		affected, err = result.RowsAffected()

		if err != nil {
			return 0, fmt.Errorf("failed to retrieve rows affected: %w", err)
		}

		if affected == 0 {
			return 0, fmt.Errorf("failed to update hash public key")
		}

		rowid, err := result.LastInsertId()
		if err != nil {
			return 0, fmt.Errorf("failed to retrieve last insert id: %w", err)
		}

		log.Info("Updating hash public key", "id", id, "charm_id", *charm_id, "connection_id", *connection_id, "affected", affected, "rowid", rowid)
	}
	if err != nil {
		return 0, fmt.Errorf("failed to execute query: %w", err)
	}
	log.Info("Updated hash public key", "id", id, "charm_id", *charm_id, "connection_id", *connection_id, "affected", affected)

	return affected, nil
}

func UpdateTextPublicKey(id int64, charm_id *string, connection_id *string) (int64, error) {
	log.Info("Updating text public key", "id", id, "charm_id", *charm_id, "connection_id", *connection_id)
	if id == 0 || (charm_id == nil && connection_id == nil) {
		return 0, fmt.Errorf("no fields to update")
	}
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
		return 0, fmt.Errorf("failed to open db %s: %w", host, err)
	}
	defer db.Close()

	stmtBase := "UPDATE text_public_key SET "
	if charm_id != nil {
		stmtBase += "charm_id = ? "
	}
	if connection_id != nil {
		if charm_id != nil {
			stmtBase += ", "
		}
		stmtBase += "connection_id = ? "
	}
	stmtBase += "WHERE rowid = ?"

	stmt, err := db.Prepare(stmtBase)
	if err != nil {
		return 0, fmt.Errorf("failed to prepare query: %w", err)
	}
	defer stmt.Close()

	var affected int64

	if charm_id != nil && connection_id != nil {
		if charm_id != nil && *charm_id == "" {
			charm_id = nil
		}
		if connection_id != nil && *connection_id == "" {
			connection_id = nil
		}
		result, err := stmt.Exec(charm_id, connection_id, id)

		if err != nil {
			return 0, fmt.Errorf("failed to execute query: %w", err)
		}

		affected, err = result.RowsAffected()

		if err != nil {
			return 0, fmt.Errorf("failed to retrieve rows affected: %w", err)
		}

		if affected == 0 {
			return 0, fmt.Errorf("failed to update text public key")
		}

		rowid, err := result.LastInsertId()

		if err != nil {
			return 0, fmt.Errorf("failed to retrieve last insert id: %w", err)
		}

		log.Info("Updating text public key", "id", id, "charm_id", *charm_id, "connection_id", *connection_id, "affected", affected, "rowid", rowid)
	} else if charm_id != nil {
		if charm_id != nil && *charm_id == "" {
			charm_id = nil
		}
		result, err := stmt.Exec(charm_id, id)

		if err != nil {
			return 0, fmt.Errorf("failed to execute query: %w", err)
		}

		affected, err = result.RowsAffected()

		if err != nil {
			return 0, fmt.Errorf("failed to retrieve rows affected: %w", err)
		}

		if affected == 0 {
			return 0, fmt.Errorf("failed to update text public key")
		}

		rowid, err := result.LastInsertId()

		if err != nil {
			return 0, fmt.Errorf("failed to retrieve last insert id: %w", err)
		}

		log.Info("Updating text public key", "id", id, "charm_id", *charm_id, "connection_id", *connection_id, "affected", affected, "rowid", rowid)
	} else if connection_id != nil {
		if connection_id != nil && *connection_id == "" {
			connection_id = nil
		}
		result, err := stmt.Exec(connection_id, id)

		if err != nil {
			return 0, fmt.Errorf("failed to execute query: %w", err)
		}

		affected, err = result.RowsAffected()

		if err != nil {
			return 0, fmt.Errorf("failed to retrieve rows affected: %w", err)
		}

		rowid, err := result.LastInsertId()

		if err != nil {
			return 0, fmt.Errorf("failed to retrieve last insert id: %w", err)
		}

		log.Info("Updating text public key", "id", id, "charm_id", *charm_id, "connection_id", *connection_id, "affected", affected, "rowid", rowid)
	}
	if err != nil {
		return 0, fmt.Errorf("failed to execute query: %w", err)
	}
	log.Info("Updated text public key", "id", id, "charm_id", *charm_id, "connection_id", *connection_id, "affected", affected)

	return affected, nil
}
