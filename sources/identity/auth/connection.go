package auth

import (
	"database/sql"
	"encoding/json"
	"fmt"
	"os"
	"time"

	"github.com/charmbracelet/log"
	"github.com/nrednav/cuid2"
)

type Connection struct {
	ConnectionID               *string
	Status                     *string
	CharmID                    *string
	TargetID                   *string
	App                        *string
	AuthMethod                 *string
	Type                       *string
	Name                       *string
	Description                *string
	Username                   *string
	PasswordLength             *int64
	PasswordHash               *string
	PasswordHashType           *string
	PublicKey                  *string
	Interactive                *string
	Pty                        *string
	Protocol                   *string
	ServerVersion              *string
	ClientVersion              *string
	SessionHash                *string
	PermissionsCriticalOptions *string
	PermissionsExtensions      *string
	Admin                      *string
	Query                      *string
	Host                       *string
	Port                       int64
	Commands                   *string
	Comments                   *string
	History                    *string
	RemoteAddr                 *string
	RemoteAddrNetwork          *string
	OpenedAt                   *time.Time
	ClosedAt                   *time.Time
	CreatedAt                  *time.Time
	UpdatedAt                  *time.Time
	DeletedAt                  *time.Time
}

type ConnectionData struct {
	ConnectionID               string `json:"connection_id"`
	Status                     string `json:"status"`
	CharmID                    string `json:"charm_id"`
	TargetID                   string `json:"target_id"`
	App                        string `json:"app"`
	AuthMethod                 string `json:"auth_method"`
	Type                       string `json:"type"`
	Name                       string `json:"name"`
	Description                string `json:"description"`
	Username                   string `json:"username"`
	PasswordLength             int64  `json:"password_length"`
	PasswordHash               string `json:"password_hash"`
	PasswordHashType           string `json:"password_hash_type"`
	PublicKey                  string `json:"public_key"`
	Interactive                string `json:"interactive"`
	Pty                        string `json:"pty"`
	Protocol                   string `json:"protocol"`
	ServerVersion              string `json:"server_version"`
	ClientVersion              string `json:"client_version"`
	SessionHash                string `json:"session_hash"`
	PermissionsCriticalOptions string `json:"permissions_critical_options"`
	PermissionsExtensions      string `json:"permissions_extensions"`
	Admin                      string `json:"admin"`
	Query                      string `json:"query"`
	Host                       string `json:"host"`
	Port                       int64  `json:"port"`
	Commands                   string `json:"commands"`
	Comments                   string `json:"comments"`
	History                    string `json:"history"`
	RemoteAddr                 string `json:"remote_addr"`
	RemoteAddrNetwork          string `json:"remote_addr_network"`
	OpenedAt                   string `json:"opened_at"`
	ClosedAt                   string `json:"closed_at"`
	CreatedAt                  string `json:"created_at"`
	UpdatedAt                  string `json:"updated_at"`
	DeletedAt                  string `json:"deleted_at"`
}

func (c *Connection) ToData() ConnectionData {
	derefString := func(s *string) string {
		if s == nil {
			return ""
		}
		return *s
	}
	derefTime := func(t *time.Time) string {
		if t == nil {
			return ""
		}
		return t.Format(time.RFC3339)
	}
	derefInt64 := func(i *int64) int64 {
		if i == nil {
			return 0
		}
		return *i
	}
	return ConnectionData{
		ConnectionID:               derefString(c.ConnectionID),
		Status:                     derefString(c.Status),
		CharmID:                    derefString(c.CharmID),
		TargetID:                   derefString(c.TargetID),
		App:                        derefString(c.App),
		AuthMethod:                 derefString(c.AuthMethod),
		Type:                       derefString(c.Type),
		Name:                       derefString(c.Name),
		Description:                derefString(c.Description),
		Username:                   derefString(c.Username),
		PasswordLength:             derefInt64(c.PasswordLength),
		PasswordHash:               derefString(c.PasswordHash),
		PasswordHashType:           derefString(c.PasswordHashType),
		PublicKey:                  derefString(c.PublicKey),
		Interactive:                derefString(c.Interactive),
		Pty:                        derefString(c.Pty),
		Protocol:                   derefString(c.Protocol),
		ServerVersion:              derefString(c.ServerVersion),
		ClientVersion:              derefString(c.ClientVersion),
		SessionHash:                derefString(c.SessionHash),
		PermissionsCriticalOptions: derefString(c.PermissionsCriticalOptions),
		PermissionsExtensions:      derefString(c.PermissionsExtensions),
		Admin:                      derefString(c.Admin),
		Query:                      derefString(c.Query),
		Host:                       derefString(c.Host),
		Port:                       c.Port,
		Commands:                   derefString(c.Commands),
		Comments:                   derefString(c.Comments),
		History:                    derefString(c.History),
		RemoteAddr:                 derefString(c.RemoteAddr),
		RemoteAddrNetwork:          derefString(c.RemoteAddrNetwork),
		OpenedAt:                   derefTime(c.OpenedAt),
		ClosedAt:                   derefTime(c.ClosedAt),
		CreatedAt:                  derefTime(c.CreatedAt),
		UpdatedAt:                  derefTime(c.UpdatedAt),
		DeletedAt:                  derefTime(c.DeletedAt),
	}
}

func (b *Connection) String() string {
	jsonB, err := json.Marshal(b.ToData())
	if err != nil {
		return fmt.Sprintf("failed to marshal connection: %v", err)
	}
	return string(jsonB)
}

func (b *Connection) SetPermissionsExtensions(permissionsExtensions string) error {
	err := b.Update("permissions_extensions", permissionsExtensions)
	if err != nil {
		log.Error("Failed to update permissions extensions", "error", err)
		return err
	}

	b.PermissionsExtensions = &permissionsExtensions
	return nil
}

func (b *Connection) SetStatus(status string) error {
	err := b.Update("status", status)
	if err != nil {
		log.Error("Failed to update status", "error", err)
		return err
	}

	b.Status = &status
	return nil
}

func (b *Connection) SetTargetID(targetID string) error {
	err := b.Update("target_id", targetID)
	if err != nil {
		log.Error("Failed to update target id", "error", err)
		return err
	}

	b.TargetID = &targetID
	return nil
}

func (b *Connection) SetConnectionID(connectionID string) error {
	err := b.Update("connection_id", connectionID)
	if err != nil {
		log.Error("Failed to update connection id", "error", err)
		return err
	}

	b.ConnectionID = &connectionID
	return nil
}

func (b *Connection) SetCharmID(charmID string) error {
	err := b.Update("charm_id", charmID)
	if err != nil {
		log.Error("Failed to update charm id", "error", err)
		return err
	}

	b.CharmID = &charmID
	return nil
}

func (b *Connection) SetApp(app string) error {
	err := b.Update("app", app)
	if err != nil {
		log.Error("Failed to update app", "error", err)
		return err
	}

	b.App = &app
	return nil
}

func (b *Connection) SetType(typeValue string) error {
	err := b.Update("type", typeValue)
	if err != nil {
		log.Error("Failed to update type", "error", err)
		return err
	}

	b.Type = &typeValue
	return nil
}

func (b *Connection) SetName(name string) error {
	err := b.Update("name", name)
	if err != nil {
		log.Error("Failed to update name", "error", err)
		return err
	}

	b.Name = &name
	return nil
}

func (b *Connection) SetDescription(description string) error {
	err := b.Update("description", description)
	if err != nil {
		log.Error("Failed to update description", "error", err)
		return err
	}

	b.Description = &description
	return nil
}

func (b *Connection) SetUsername(username string) error {
	err := b.Update("username", username)
	if err != nil {
		log.Error("Failed to update username", "error", err)
		return err
	}

	b.Username = &username
	return nil
}

func (b *Connection) SetAdmin(admin string) error {
	err := b.Update("admin", admin)
	if err != nil {
		log.Error("Failed to update admin", "error", err)
		return err
	}

	b.Admin = &admin
	return nil
}

func (b *Connection) SetQuery(query string) error {
	err := b.Update("query", query)
	if err != nil {
		log.Error("Failed to update query", "error", err)
		return err
	}

	b.Query = &query
	return nil
}

func (b *Connection) SetCommands(commands string) error {
	err := b.Update("commands", commands)
	if err != nil {
		log.Error("Failed to update commands", "error", err)
		return err
	}

	b.Commands = &commands
	return nil
}

func (b *Connection) SetComments(comments string) error {
	err := b.Update("comments", comments)
	if err != nil {
		log.Error("Failed to update comments", "error", err)
		return err
	}

	b.Comments = &comments
	return nil
}

func (b *Connection) Update(column string, value interface{}) error {
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

	stmt, err := db.Prepare(fmt.Sprintf("UPDATE connection SET %s = ?, updated_at = datetime('now'), history = CONCAT(history, ?) WHERE connection_id = ?", column))
	if err != nil {
		return fmt.Errorf("failed to prepare update statement: %w", err)
	}
	defer stmt.Close()

	result, err := stmt.Exec(value, "\n"+column+": "+fmt.Sprintf("%v", value), *b.ConnectionID)
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

func (b *Connection) Insert() (*string, error) {
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
		INSERT INTO connection (
			connection_id,
			status,
			charm_id,
			target_id,
			app,
			auth_method,
			type,
			name,
			description,
			username,
			password_length,
			password_hash,
			password_hash_type,
			public_key,
			interactive,
			pty,
			protocol,
			server_version,
			client_version,
			session_hash,
			permissions_critical_options,
			permissions_extensions,
			admin,
			query,
			host,
			port,
			commands,
			comments,
			history,
			remote_addr,
			remote_addr_network,
			opened_at,
			closed_at,
			deleted_at
		) VALUES (
			?,
			?,
			?,
			?,
			?,
			?,
			?,
			?,
			?,
			?,
			?,
			?,
			?,
			?,
			?,
			?,
			?,
			?,
			?,
			?,
			?,
			?,
			?,
			?,
			?,
			?,
			?,
			?,
			?,
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

	jsonB, err := json.Marshal(b)
	if err != nil {
		return nil, fmt.Errorf("failed to marshal initial history: %w", err)
	}

	bHistory := ""
	if b.History != nil {
		bHistory = *b.History
	}
	history := bHistory + "\n" + string(jsonB)

	result, err := stmt.Exec(
		b.ConnectionID,
		b.Status,
		b.CharmID,
		b.TargetID,
		b.App,
		b.AuthMethod,
		b.Type,
		b.Name,
		b.Description,
		b.Username,
		b.PasswordLength,
		b.PasswordHash,
		b.PasswordHashType,
		b.PublicKey,
		b.Interactive,
		b.Pty,
		b.Protocol,
		b.ServerVersion,
		b.ClientVersion,
		b.SessionHash,
		b.PermissionsCriticalOptions,
		b.PermissionsExtensions,
		b.Admin,
		b.Query,
		b.Host,
		b.Port,
		b.Commands,
		b.Comments,
		history,
		b.RemoteAddr,
		b.RemoteAddrNetwork,
		b.OpenedAt,
		b.ClosedAt,
		b.DeletedAt,
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