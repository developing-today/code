package auth

import (
	"database/sql"
	"fmt"
	"os"
	"time"

	"github.com/charmbracelet/log"
	"github.com/nrednav/cuid2"
)

type KeyPair struct {
	CharmID      *string
	ConnectionID *string
	Type         *string
	PrivateKey   *string
	PublicKey    *string
	CreatedAt    time.Time
	UpdatedAt    time.Time
	DeletedAt    time.Time
}

func (b *KeyPair) Sync() {
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
		log.Fatal("failed to open db", "error", err)
	}
	defer db.Close()

	stmt, err := db.Prepare("SELECT charm_id, connection_id, type, private_key, public_key, created_at, updated_at, deleted_at FROM private_key WHERE public_key = ?")
	if err != nil {
		log.Fatal("failed to prepare query", "error", err)
	}
	defer stmt.Close()

	err = stmt.QueryRow(b.PublicKey).Scan(&b.CharmID, &b.ConnectionID, &b.Type, &b.PrivateKey, &b.PublicKey, &b.CreatedAt, &b.UpdatedAt, &b.DeletedAt)

	if err != nil {
		log.Fatal("failed to execute query", "error", err)
	}

	log.Info("Connection synced", "connection_id", *b.ConnectionID)
}

func (b *KeyPair) SetConnectionID(connectionID string) error {
	err := b.Update("connection_id", connectionID)
	if err != nil {
		log.Error("Failed to update connection id", "error", err)
		return err
	}

	b.ConnectionID = &connectionID
	return nil
}

func (b *KeyPair) SetCharmID(charmID string) error {
	err := b.Update("charm_id", charmID)
	if err != nil {
		log.Error("Failed to update charm id", "error", err)
		return err
	}

	b.CharmID = &charmID
	return nil
}

func (b *KeyPair) SetType(typeStr string) error {
	err := b.Update("type", typeStr)
	if err != nil {
		log.Error("Failed to update type", "error", err)
		return err
	}

	b.Type = &typeStr
	return nil
}

func (b *KeyPair) Update(column string, value any) error {
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
		return fmt.Errorf("failed to open db %s: %w", host, err)
	}
	defer db.Close()

	stmt, err := db.Prepare(fmt.Sprintf("UPDATE private_key SET %s = ?, updated_at = NOW(),WHERE public_key = ?", column))
	if err != nil {
		return fmt.Errorf("failed to prepare update statement: %w", err)
	}
	defer stmt.Close()

	result, err := stmt.Exec(value, b.PublicKey)
	if err != nil {
		return fmt.Errorf("failed to execute update: %w", err)
	}

	affected, err := result.RowsAffected()
	if err != nil {
		return fmt.Errorf("failed to retrieve rows affected: %w", err)
	}

	log.Info("Connection updated", "connection_id", *b.ConnectionID, "affected", affected)

	return nil

}

func (b *KeyPair) Insert() (*string, error) {
	if b.ConnectionID != nil {
		return nil, fmt.Errorf("inserting connection with existing connection id")
	}
	connectionID := cuid2.Generate()
	b.ConnectionID = &connectionID

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

	stmt, err := db.Prepare(`
		INSERT INTO private_key (
			charm_id,
			connection_id,
			type,
			private_key,
			public_key
		) VALUES (
			?,
			?,
			?,
			?,
			?
		)
	`)
	if err != nil {
		return nil, fmt.Errorf("failed to prepare insert statement: %w", err)
	}
	defer stmt.Close()
	result, err := stmt.Exec(
		b.CharmID,
		b.ConnectionID,
		b.Type,
		b.PrivateKey,
		b.PublicKey,
	)

	if err != nil {
		return nil, fmt.Errorf("failed to execute insert: %w", err)
	}

	insertedId, err := result.LastInsertId()
	if err != nil {
		return nil, fmt.Errorf("failed to retrieve last insert id: %w", err)
	}

	log.Info("Connection inserted", "connection_id", connectionID, "insertedId", insertedId)

	return &connectionID, nil
}
