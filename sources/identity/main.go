package main

import (
	"context"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"

	"github.com/charmbracelet/log"
	"github.com/charmbracelet/melt"
	"github.com/charmbracelet/promwish"
	"github.com/charmbracelet/ssh"
	"github.com/charmbracelet/wish"
	"github.com/charmbracelet/wish/comment"
	elapsed "github.com/charmbracelet/wish/elapsed"
	"github.com/charmbracelet/wish/logging"
	"github.com/charmbracelet/wish/scp"
	"github.com/developing-today/code/src/identity/auth"
	"github.com/knadh/koanf"
	"github.com/knadh/koanf/parsers/kdl"
	"github.com/knadh/koanf/providers/file"
	"github.com/spf13/cobra"
	gossh "golang.org/x/crypto/ssh"
)

var separator = "."
var configuration = koanf.New(separator)
var configurationPaths = []string{
	"./config.kdl",
}
var generatedKeyDirPath = ".ssh/generated"
var hostKeyPath = ".ssh/term_info_ed25519"

func loadConfiguration() {
	for _, path := range configurationPaths {
		if _, err := os.Stat(path); err == nil {
			if err := configuration.Load(file.Provider(path), kdl.Parser()); err != nil {
				log.Error("Failed to load config", "error", err)
			} else {
				log.Info("Loaded config", "path", path)
			}
		} else {
			log.Info("Config not found", "path", path)
		}
	}
}

func initializeConfiguration() {
	configuration = koanf.New(separator)
}

func initializeAndLoadConfiguration() {
	initializeConfiguration()
	log.Debug("Initialized config", "config", configuration.Sprint())
	loadConfiguration()
	log.Info("Loaded config", "config", configuration.Sprint())
}

func init() {
	cobra.OnInitialize(initializeAndLoadConfiguration)
	rootCmd.AddCommand(startCmd)
}

var rootCmd = &cobra.Command{
	Use:   "identity",
	Short: "publish your identity",
	Long:  `publish your identity and allow others to connect to you.`,
}

var startCmd = &cobra.Command{
	Use:     "start",
	Short:   "Starts the identity server",
	Run:     start,
	Aliases: []string{"s", "run", "serve", "publish", "pub", "p", "i", "y", "u", "o", "p", "q", "w", "e", "r", "t", "a", "s", "d", "f", "g", "h", "j", "k", "l", "z", "x", "c", "v", "b"},
}

func main() {
	if err := rootCmd.Execute(); err != nil {
		fmt.Println(err)
		os.Exit(1)
	}
}

func start(cmd *cobra.Command, args []string) {
	handler := scp.NewFileSystemHandler("./files")
	s, err := wish.NewServer(
		wish.WithMiddleware(
			scp.Middleware(handler, handler),
			func(h ssh.Handler) ssh.Handler {
				return func(s ssh.Session) {
					h(s)
				}
			},
			comment.Middleware("Thanks, have a nice day!"),
			elapsed.Middleware(),
			promwish.Middleware("0.0.0.0:9222", "identity"),
			logging.Middleware(),
		),
		wish.WithPasswordAuth(func(ctx ssh.Context, password string) bool {
			log.Info("Accepting password", "password", password, "len", len(password))
			return Connect(ctx, nil, &password, nil)
		}),
		wish.WithKeyboardInteractiveAuth(func(ctx ssh.Context, challenge gossh.KeyboardInteractiveChallenge) bool {
			log.Info("Accepting keyboard interactive")
			return Connect(ctx, nil, nil, challenge)
		}),
		wish.WithPublicKeyAuth(func(ctx ssh.Context, key ssh.PublicKey) bool {
			log.Info("Accepting public key", "publicKeyType", key.Type(), "publicKeyString", base64.StdEncoding.EncodeToString(key.Marshal()))
			return Connect(ctx, key, nil, nil)
		}),
		wish.WithBannerHandler(func(ctx ssh.Context) string {
			return `
Welcome to the identity server!

By using this service, you agree to the following terms and conditions:

- EACH PARTY MAKES NO WARRANTIES, EXPRESS, IMPLIED OR OTHERWISE, REGARDING ITS ACCURACY, COMPLETENESS OR PERFORMANCE.

- THE SERVICE AND ANY RELATED SERVICES ARE PROVIDED ON AN "AS IS" AND "AS AVAILABLE" BASIS, WITHOUT WARRANTY OF ANY KIND, WHETHER WRITTEN OR ORAL, EXPRESS OR IMPLIED.

- TO THE FULL EXTENT PERMISSIBLE BY LAW, DEVELOPING.TODAY LLC WILL NOT BE LIABLE FOR ANY DAMAGES OF ANY KIND ARISING FROM THE USE OF ANY DEVELOPING.TODAY LLC SERVICE, OR FROM ANY INFORMATION, CONTENT, MATERIALS, PRODUCTS (INCLUDING SOFTWARE) OR OTHER SERVICES INCLUDED ON OR OTHERWISE MADE AVAILABLE TO YOU THROUGH ANY DEVELOPING.TODAY LLC SERVICE, INCLUDING, BUT NOT LIMITED TO DIRECT, INDIRECT, INCIDENTAL, PUNITIVE, AND CONSEQUENTIAL DAMAGES, UNLESS OTHERWISE SPECIFIED IN WRITING.

If you do not agree to these terms and conditions, you may not use this service and must disconnect immediately.

` + fmt.Sprintf("You are using the identity server at %s:%d\n", configuration.String("host"), configuration.Int("port")) + `
` + fmt.Sprintf("You are connecting from %s\n", ctx.RemoteAddr().String()) + `
` + fmt.Sprintf("Your connection id is %s\n", ctx.Permissions().Extensions["connection-id"]) + `
` + fmt.Sprintf("Your room is %s\n", ctx.Permissions().Extensions["room"]) + `
` + fmt.Sprintf("Your public key is %s %s\n", ctx.Permissions().Extensions["public-key-type"], ctx.Permissions().Extensions["public-key"])
		}),
		wish.WithAddress(fmt.Sprintf("%s:%d", configuration.String("host"), configuration.Int("port"))),
		wish.WithHostKeyPath(hostKeyPath),
	)
	if err != nil {
		log.Error("could not start server", "error", err)
		return
	}

	done := make(chan os.Signal, 1)
	signal.Notify(done, os.Interrupt, syscall.SIGINT, syscall.SIGTERM)
	log.Info("Starting SSH server", "host", configuration.String("host"), "port", configuration.Int("port"))
	go func() {
		if err := s.ListenAndServe(); err != nil && !errors.Is(err, ssh.ErrServerClosed) {
			log.Error("could not start server", "error", err)
			done <- os.Interrupt
		}
	}()

	<-done
	log.Info("Stopping SSH server")
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	if err := s.Shutdown(ctx); err != nil && !errors.Is(err, ssh.ErrServerClosed) {
		log.Error("could not stop server", "error", err)
	}
}

type Challenge struct {
	Name        string
	Instruction string
	Questions   []string
	Answers     []string
}

func Connect(ctx ssh.Context, key ssh.PublicKey, password *string, challenge gossh.KeyboardInteractiveChallenge) bool {
	status := "open"
	app := "identity"
	connectionType := "ssh"
	user := ctx.User()

	if ctx.Permissions().Extensions == nil {
		ctx.Permissions().Extensions = make(map[string]string)
	}

	var interactive *string

	if challenge != nil {

		challengeName := "Room Challenge:"
		instruction := "Select your room and enter the password if required."
		questions := []string{"What is the room? ", "What is the password? (leave blank if none, password is sometimes required. Passwords are insecure, passwords may be visible to others.)"}
		show := []bool{true, false}
		answers, err := challenge(challengeName, instruction, questions, show)
		if err != nil {
			log.Error("Failed to get keyboard interactive response", "error", err)
			return false
		}
		ctx.Permissions().Extensions["room"] = answers[0]
		password = &answers[1]

		challengesJson, err := json.Marshal([]Challenge{
			{
				Name:        challengeName,
				Instruction: instruction,
				Questions:   questions,
				Answers:     answers,
			},
		})
		if err != nil {
			log.Error("Failed to marshal challenges", "error", err)
			return false
		}
		interactiveStr := string(challengesJson)
		interactive = &interactiveStr

		log.Info("Accepting keyboard interactive", "response", answers, "room", answers[0])
	}

	var passwordLength *int64
	var passwordHash *string
	var passwordHashType *string
	var passwordSha256 []byte

	if password != nil {
		log.Info("Accepting password", "password", *password, "len", len(*password))
		passwordLength = new(int64)
		*passwordLength = int64(len(*password))
		hasher := sha256.New()
		hasher.Write([]byte(*password))
		passwordSha256 = hasher.Sum(nil)
		passwordHashStr := base64.StdEncoding.EncodeToString(passwordSha256)
		passwordHash = &passwordHashStr
		if ctx.Permissions().Extensions == nil {
			ctx.Permissions().Extensions = make(map[string]string)
		}
		ctx.Permissions().Extensions["password-hash"] = *passwordHash

		passwordHashTypeStr := "sha256"
		passwordHashType = &passwordHashTypeStr
		ctx.Permissions().Extensions["password-hash-type"] = *passwordHashType

		log.Info("Accepting password", "passwordHash", *passwordHash)
	}

	var publicKey *string
	var publicKeyType string

	if key != nil {
		log.Info("Accepting public key", "publicKeyType", key.Type(), "publicKeyString", base64.StdEncoding.EncodeToString(key.Marshal()))
		publicKeyStr := base64.StdEncoding.EncodeToString(key.Marshal())
		publicKey = &publicKeyStr
		publicKeyType = key.Type()
		ctx.Permissions().Extensions["public-key"] = *publicKey
		ctx.Permissions().Extensions["public-key-type"] = publicKeyType
	}
	if publicKey == nil {
		log.Info("No public key provided, generating one")
		if password == nil || passwordLength == nil || passwordHash == nil || passwordHashType == nil || passwordSha256 == nil {
			log.Error("No public key or password provided", "password", *password, "passwordLength", *passwordLength, "passwordHash", *passwordHash, "passwordHashType", *passwordHashType, "passwordSha256", passwordSha256)
			return false
		}
		pk := ed25519.NewKeyFromSeed(passwordSha256) // need to put these in text instead, put random into regular

		pubKey := pk.Public().(ed25519.PublicKey)
		sshPubKey, err := gossh.NewPublicKey(pubKey)
		if err != nil {
			log.Fatal("Failed to create SSH public key", err)
		}
		authorizedKey := gossh.MarshalAuthorizedKey(sshPubKey)
		authKey := string(authorizedKey)
		log.Info("Generated public key", "authKey", authKey, "authorizedKey", authorizedKey, "sshPubKey", sshPubKey, "pubKey", pubKey, "pk", pk, "pkLen", len(pk))

		// Encode to base64
		publicKeyStr := base64.StdEncoding.EncodeToString(gossh.MarshalAuthorizedKey(sshPubKey))
		log.Info("Generated public key", "publicKeyStr", publicKeyStr)

		publicKey = &publicKeyStr
		publicKeyType = "ed25519"
		log.Info("Generated public key", "publicKey", *publicKey)
		parts := strings.Fields(string(authorizedKey))
		if len(parts) < 2 {
			log.Fatal("Invalid public key format")
		}
		keyData, err := base64.StdEncoding.DecodeString(parts[1])
		if err != nil {
			log.Fatal("Failed to decode base64 public key", err)
		}
		log.Info("Generated public key, preparing", "keyData", keyData, "keyDataLen", len(keyData), "parts", parts, "publicKey", *publicKey)

		out, comment, options, rest, err := gossh.ParseAuthorizedKey(authorizedKey)
		if err != nil {
			log.Fatal("Failed to parse public key", "error", err)
		}
		log.Info("Parsed public key", "out", out, "comment", comment, "options", options, "rest", rest)
		key = out
		log.Info("Generated public key", "publicKey", publicKeyStr)
		ctx.Permissions().Extensions["public-key"] = *publicKey
		ctx.Permissions().Extensions["public-key-type"] = publicKeyType
		pkMelted, err := melt.ToMnemonic(&pk)
		if err != nil {
			log.Error("Failed to melt private key", "error", err)
			return false
		}
		ctx.Permissions().Extensions["private-key-melted"] = pkMelted
		log.Info("Melted private key", "pkMelted", pkMelted)
	}

	if publicKey == nil {
		log.Error("No public key provided")
		return false
	}

	authorizedKey := gossh.MarshalAuthorizedKey(key)
	log.Info("Public key used", "publicKey", authorizedKey)

	serverVersion := ctx.ServerVersion()
	clientVersion := ctx.ClientVersion()
	sessionHash := ctx.SessionID()
	permissionsCriticalOptionsJson, err := json.Marshal(ctx.Permissions().CriticalOptions)
	if err != nil {
		log.Error("Failed to marshal critical options", "error", err)
		return false
	}
	permissionsCriticalOptions := string(permissionsCriticalOptionsJson)
	host := ctx.LocalAddr().String()
	port := int64(ctx.LocalAddr().(*net.TCPAddr).Port)
	remoteAddr := ctx.RemoteAddr().String()
	remoteAddrNetwork := ctx.RemoteAddr().Network()
	openedAt := time.Now()
	pty := ""
	protocol := "ssh"
	permissionsExtensions := ""
	admin := ""
	query := ""
	commands := ""
	comments := ""
	history := ""

	log.Info("Connection opened", "openedAt", openedAt, "remoteAddr", remoteAddr, "remoteAddrNetwork", remoteAddrNetwork, "host", host, "port", port, "serverVersion", serverVersion, "clientVersion", clientVersion, "sessionHash", sessionHash, "permissionsCriticalOptions", permissionsCriticalOptions)
	connection := auth.Connection{
		Status:                     &status,
		Name:                       &user,
		Description:                &user,
		App:                        &app,
		Type:                       &connectionType,
		Username:                   &user,
		PublicKey:                  publicKey,
		ServerVersion:              &serverVersion,
		ClientVersion:              &clientVersion,
		SessionHash:                &sessionHash,
		PermissionsCriticalOptions: &permissionsCriticalOptions,
		PermissionsExtensions:      &permissionsExtensions,
		Host:                       &host,
		Port:                       port,
		Pty:                        &pty,
		Protocol:                   &protocol,
		RemoteAddr:                 &remoteAddr,
		RemoteAddrNetwork:          &remoteAddrNetwork,
		OpenedAt:                   &openedAt,
		Interactive:                interactive,
		PasswordLength:             passwordLength,
		PasswordHash:               passwordHash,
		PasswordHashType:           passwordHashType,
		Admin:                      &admin,
		Query:                      &query,
		Commands:                   &commands,
		Comments:                   &comments,
		History:                    &history,
	}

	log.Info("Inserting connection", "connection", connection.ToData())
	connectionID, err := connection.Insert()

	if err != nil {
		log.Error("Failed to insert connection", "error", err)
		return false
	}
	log.Info("Inserted connection", "connectionID", &connectionID, "connection", connection)
	ctx.Permissions().Extensions["connection-id"] = fmt.Sprintf("%d", &connectionID)

	permissionsExtensionsJson, err := json.Marshal(ctx.Permissions().Extensions)
	if err != nil {
		log.Error("Failed to marshal extensions", "error", err)
		return false
	}
	log.Info("Setting permissions extensions", "permissionsExtensions", string(permissionsExtensionsJson))
	connection.SetPermissionsExtensions(string(permissionsExtensionsJson))

	log.Info("Checking public key", "publicKey", *publicKey)
	result, err := auth.CheckPublicKey(ctx, key)

	log.Info("Checked public key", "result", result, "error", err)
	if err != nil {
		var userID int64
		userID, err = auth.InsertUser(ctx)
		if err != nil {
			log.Error("Failed to insert user", "error", err)
			return false
		}
		log.Info("Inserted user", "userID", userID)

		var pk int64
		pk, err = auth.InsertPublicKey(userID, key)
		if err != nil {
			log.Error("Failed to insert public key", "error", err)
			return false
		}
		log.Info("Inserted public key", "pk", pk)

		result, err = auth.CheckPublicKey(ctx, key)

		log.Info("Checked public key", "result", result, "error", err)
	} else {
		log.Info("Public key already exists", "result", result)
	}
	if err != nil {
		log.Error("Failed to check public key", "error", err)
		return false
	}
	ctx.Permissions().Extensions["charm-id"] = result.ID
	ctx.Permissions().Extensions["charm-name"] = result.Name
	log.Info("Setting permissions extensions", "charm-id", result.ID, "charm-name", result.Name)
	jsonRoles, err := json.Marshal(result.Roles)
	if err != nil {
		log.Error("Failed to marshal roles", "error", err)
		return false
	}
	log.Info("Setting permissions extensions", "charm-roles", string(jsonRoles))
	ctx.Permissions().Extensions["charm-roles"] = string(jsonRoles)
	ctx.Permissions().Extensions["charm-created-at"] = result.CreatedAt.Format(time.RFC3339)
	ctx.Permissions().Extensions["charm-public-key-created-at"] = result.PublicKeyCreatedAt.Format(time.RFC3339)
	ctx.Permissions().Extensions["charm-public-key-type"] = result.PublicKeyType
	ctx.Permissions().Extensions["charm-public-key"] = result.PublicKeyString

	log.Info("Setting permissions extensions", "charm-created-at", result.CreatedAt.Format(time.RFC3339), "charm-public-key-created-at", result.PublicKeyCreatedAt.Format(time.RFC3339), "charm-public-key-type", result.PublicKeyType, "charm-public-key", result.PublicKeyString)

	return true
}
