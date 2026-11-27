package cli

import (
	"context"
	"errors"
	"fmt"
	"os"
	"strings"
	"text/tabwriter"
	"time"

	"github.com/developing-today/code/pkgs/mcpx/internal/daemon"
)

// CmdAuth manages local daemon authentication credentials. It deliberately
// reads and writes the state directory directly: there is no HTTP path for
// provisioning or enumerating keys.
func (a *App) CmdAuth(_ context.Context, args []string) error {
	if len(args) == 0 {
		return a.helpFor("auth")
	}
	if args[0] != "key" {
		return fmt.Errorf("no auth subcommand %q; key", args[0])
	}
	return a.cmdAuthKey(args[1:])
}

func (a *App) cmdAuthKey(args []string) error {
	if len(args) == 0 || isHelpToken(args[0]) {
		return a.helpFor("auth")
	}
	sub, rest := args[0], args[1:]
	switch sub {
	case "create":
		return a.authKeyCreate(rest)
	case "list":
		return a.authKeyList(rest)
	case "revoke":
		return a.authKeyRevoke(rest)
	default:
		return fmt.Errorf("no auth key subcommand %q; create, list or revoke", sub)
	}
}

type authKeyPublic struct {
	ID            string     `json:"id"`
	User          string     `json:"user"`
	Label         string     `json:"label"`
	Ceiling       string     `json:"ceiling"`
	CapabilityRef string     `json:"capability_ref,omitempty"`
	CreatedAt     time.Time  `json:"created_at"`
	LastUsedAt    *time.Time `json:"last_used_at,omitempty"`
	Revoked       bool       `json:"revoked"`
	RevokedAt     *time.Time `json:"revoked_at,omitempty"`
}

func publicAuthKey(record daemon.KeyRecord) authKeyPublic {
	return authKeyPublic{
		ID:            record.ID,
		User:          record.User,
		Label:         record.Label,
		Ceiling:       record.Ceiling,
		CapabilityRef: record.CapabilityRef,
		CreatedAt:     record.CreatedAt,
		LastUsedAt:    record.LastUsedAt,
		Revoked:       record.Revoked,
		RevokedAt:     record.RevokedAt,
	}
}

type authKeyCreated struct {
	Key    authKeyPublic `json:"key"`
	Secret string        `json:"secret"`
}

func (a *App) authKeyCreate(args []string) error {
	fs := newFlagSet("auth key create")
	user := fs.String("user", "", "user or standing machine role that owns the key")
	label := fs.String("label", "", "operator-visible name for this key")
	ceiling := fs.String("ceiling", daemon.CeilingDiscovery, "named capability ceiling (default: discovery)")
	if err := parseFlags(a, fs, args); err != nil {
		return err
	}
	if fs.NArg() != 0 {
		return errors.New("usage: mcpx auth key create --user <user> --label <label> [--ceiling <name>]")
	}
	if strings.TrimSpace(*user) == "" || strings.TrimSpace(*label) == "" {
		return errors.New("mcpx auth key create requires --user and --label")
	}
	userValue, labelValue := strings.TrimSpace(*user), strings.TrimSpace(*label)
	ceilingValue := strings.TrimSpace(*ceiling)
	if ceilingValue == "" {
		ceilingValue = daemon.CeilingDiscovery
	}
	store := daemon.OpenKeyStore(a.Paths.State)
	secret, record, err := store.Create(daemon.KeySpec{
		User:    userValue,
		Label:   labelValue,
		Ceiling: ceilingValue,
	}, nil, time.Now())
	if err != nil {
		return err
	}
	key := publicAuthKey(record)
	if a.JSON {
		return a.out(authKeyCreated{Key: key, Secret: secret})
	}
	fmt.Printf("Created key %s for %s (%s, ceiling %s).\nSecret (shown once): %s\n",
		key.ID, key.User, key.Label, key.Ceiling, secret)
	return nil
}

func (a *App) authKeyList(args []string) error {
	fs := newFlagSet("auth key list")
	if err := parseFlags(a, fs, args); err != nil {
		return err
	}
	if fs.NArg() != 0 {
		return errors.New("usage: mcpx auth key list")
	}
	records, err := daemon.OpenKeyStore(a.Paths.State).List()
	if err != nil {
		return err
	}
	keys := make([]authKeyPublic, 0, len(records))
	for _, record := range records {
		keys = append(keys, publicAuthKey(record))
	}
	if a.JSON {
		return a.out(map[string]any{"keys": keys})
	}
	if len(keys) == 0 {
		fmt.Println("No daemon authentication keys.")
		return nil
	}
	tw := tabwriter.NewWriter(os.Stdout, 0, 0, 2, ' ', 0)
	_, _ = fmt.Fprintln(tw, "ID\tUSER\tLABEL\tCEILING\tCREATED\tLAST USED\tREVOKED")
	for _, key := range keys {
		lastUsed := "-"
		if key.LastUsedAt != nil {
			lastUsed = key.LastUsedAt.UTC().Format(time.RFC3339)
		}
		revoked := "no"
		if key.Revoked {
			revoked = "yes"
		}
		_, _ = fmt.Fprintf(tw, "%s\t%s\t%s\t%s\t%s\t%s\t%s\n",
			key.ID, key.User, key.Label, key.Ceiling,
			key.CreatedAt.UTC().Format(time.RFC3339), lastUsed, revoked)
	}
	return tw.Flush()
}

func (a *App) authKeyRevoke(args []string) error {
	fs := newFlagSet("auth key revoke")
	if err := parseFlags(a, fs, args); err != nil {
		return err
	}
	if fs.NArg() != 1 || strings.TrimSpace(fs.Arg(0)) == "" {
		return errors.New("usage: mcpx auth key revoke <id>")
	}
	id := fs.Arg(0)
	if err := daemon.OpenKeyStore(a.Paths.State).Revoke(id, time.Now()); err != nil {
		return err
	}
	if a.JSON {
		return a.out(map[string]any{"id": id, "revoked": true})
	}
	fmt.Printf("Revoked key %s. The revocation takes effect on the next request.\n", id)
	return nil
}
