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

	"github.com/charmbracelet/bubbles/key"
	"github.com/charmbracelet/bubbles/spinner"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/charmbracelet/log"
	"github.com/charmbracelet/melt"
	"github.com/charmbracelet/promwish"
	"github.com/charmbracelet/ssh"
	"github.com/charmbracelet/wish"
	"github.com/charmbracelet/wish/bubbletea"
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

type errMsg error

type model struct {
	spinner  spinner.Model
	quitting bool
	err      error
	term     string
	width    int
	height   int
}

var quitKeys = key.NewBinding(
	key.WithKeys("q", "esc", "ctrl+c"),
	key.WithHelp("", "press q to quit"),
)

func (m model) Init() tea.Cmd {
	return m.spinner.Tick
}

func (m model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {

	case tea.KeyMsg:
		if key.Matches(msg, quitKeys) {
			m.quitting = true
			return m, tea.Quit

		}
		return m, nil
	case tea.WindowSizeMsg:
		m.height = msg.Height
		m.width = msg.Width
	case errMsg:
		m.err = msg
		return m, nil

	default:
		var cmd tea.Cmd
		m.spinner, cmd = m.spinner.Update(msg)
		return m, cmd
	}
	return m, nil
}

func (m model) View() string {
	s := "Your term is %s\n"
	s += "Your window size is x: %d y: %d\n\n"

	if m.err != nil {
		return m.err.Error()
	}
	str := fmt.Sprintf(s, m.term, m.width, m.height)
	str += fmt.Sprintf("\n\n   %s Loading forever... %s\n\n", m.spinner.View(), quitKeys.Help().Desc)
	if m.quitting {
		return str + "\n"
	}
	return str
}

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

func teaHandler(s ssh.Session) (tea.Model, []tea.ProgramOption) {
	pty, _, active := s.Pty()
	if !active {
		wish.Fatalln(s, "no active terminal, skipping")
		return nil, nil
	}
	sp := spinner.New()
	sp.Spinner = spinner.Dot
	sp.Style = lipgloss.NewStyle().Foreground(lipgloss.Color("205"))
	m := model{
		spinner:  sp,
		quitting: false,
		err:      nil,
		term:     pty.Term,
		width:    pty.Window.Width,
		height:   pty.Window.Height,
	}
	return m, []tea.ProgramOption{tea.WithAltScreen()}
}

func start(cmd *cobra.Command, args []string) {
	handler := scp.NewFileSystemHandler("./files")
	s, err := wish.NewServer(
		wish.WithMiddleware(
			scp.Middleware(handler, handler),
			bubbletea.Middleware(teaHandler),
			comment.Middleware("Thanks, have a nice day!"),
			elapsed.Middleware(),
			promwish.Middleware("0.0.0.0:9222", "identity"),
			logging.Middleware(),
			func(h ssh.Handler) ssh.Handler {
				return func(s ssh.Session) {
					log.Info("Session started", "session", s, "sessionID", s.Context().SessionID(), "user", s.Context().User(), "remoteAddr", s.Context().RemoteAddr().String(), "remoteAddrNetwork", s.Context().RemoteAddr().Network(), "localAddr", s.Context().LocalAddr().String(), "localAddrNetwork", s.Context().LocalAddr().Network(), "charm-id", s.Context().Permissions().Extensions["charm-id"], "charm-name", s.Context().Permissions().Extensions["charm-name"], "charm-roles", s.Context().Permissions().Extensions["charm-roles"], "charm-created-at", s.Context().Permissions().Extensions["charm-created-at"], "charm-public-key-created-at", s.Context().Permissions().Extensions["charm-public-key-created-at"], "charm-public-key-type", s.Context().Permissions().Extensions["charm-public-key-type"], "charm-public-key", s.Context().Permissions().Extensions["charm-public-key"])
					h(s)
					log.Info("Session ended", "session", s, "sessionID", s.Context().SessionID(), "user", s.Context().User(), "remoteAddr", s.Context().RemoteAddr().String(), "remoteAddrNetwork", s.Context().RemoteAddr().Network(), "localAddr", s.Context().LocalAddr().String(), "localAddrNetwork", s.Context().LocalAddr().Network(), "charm-id", s.Context().Permissions().Extensions["charm-id"], "charm-name", s.Context().Permissions().Extensions["charm-name"], "charm-roles", s.Context().Permissions().Extensions["charm-roles"], "charm-created-at", s.Context().Permissions().Extensions["charm-created-at"], "charm-public-key-created-at", s.Context().Permissions().Extensions["charm-public-key-created-at"], "charm-public-key-type", s.Context().Permissions().Extensions["charm-public-key-type"], "charm-public-key", s.Context().Permissions().Extensions["charm-public-key"])
				}
			},
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
` + fmt.Sprintf("You are connecting from-with %s\n", ctx.RemoteAddr().Network()) + `
` + fmt.Sprintf("You are connecting to %s\n", ctx.LocalAddr().String()) + `
` + fmt.Sprintf("You are connecting to-with %s\n", ctx.LocalAddr().Network()) + `
` + fmt.Sprintf("Your server version is %s\n", ctx.ServerVersion()) + `
` + fmt.Sprintf("Your client version is %s\n", ctx.ClientVersion()) + `
` + fmt.Sprintf("Your session id is %s\n", ctx.SessionID()) + `
` + fmt.Sprintf("You are connecting with user %s\n", ctx.User())
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
	Questions   []Question
}

type Question struct {
	Question   string
	Answer     string
	HideAnswer bool
}

func (c Challenge) ExecuteMutable(challenge gossh.KeyboardInteractiveChallenge) ([]string, error) {
	var questions []string
	var showAnswers []bool
	for _, question := range c.Questions {
		questions = append(questions, question.Question)
		showAnswers = append(showAnswers, !question.HideAnswer)
	}
	answers, err := challenge(c.Name, c.Instruction, questions, showAnswers)
	if err != nil {
		return nil, err
	}
	for i, answer := range answers {
		c.Questions[i].Answer = answer
	}
	return answers, nil
}

func Connect(ctx ssh.Context, key ssh.PublicKey, password *string, challenge gossh.KeyboardInteractiveChallenge) bool {
	status := "open"
	app := "identity"
	connectionType := "ssh"
	user := ctx.User()
	var authMethod string

	if key != nil {
		authMethod = "public-key"
	} else if password != nil {
		authMethod = "password"
	} else if challenge != nil {
		authMethod = "keyboard-interactive"
	} else {
		log.Error("No authentication method provided")
		return false
	}

	if ctx.Permissions().Extensions == nil {
		ctx.Permissions().Extensions = make(map[string]string)
	}

	var interactive *string

	if challenge != nil {
		c := Challenge{
			Name:        "Room Challenge:",
			Instruction: "Select your room and enter the password if required.",
			Questions: []Question{
				{
					Question: "What is the room? ",
					Answer:   "",
				},
				{
					Question:   "What is the password? (leave blank if none, password is sometimes required. Passwords are insecure, passwords may be visible to others.) ",
					Answer:     "",
					HideAnswer: true,
				},
			},
		}
		_, err := c.ExecuteMutable(challenge)
		if err != nil {
			log.Error("Failed to get keyboard interactive response", "error", err)
			return false
		}
		ctx.Permissions().Extensions["room"] = c.Questions[0].Answer
		password = &c.Questions[1].Answer

		challengesJson, err := json.Marshal(c)
		if err != nil {
			log.Error("Failed to marshal challenges", "error", err)
			return false
		}
		interactiveStr := string(challengesJson)
		interactive = &interactiveStr

		log.Info("Accepting keyboard interactive", "response", interactiveStr, "len", len(interactiveStr))
	}

	var passwordLength *int64
	var passwordHash *string
	var passwordHashType *string
	var passwordSha256 []byte
	var passwordSha256Str string

	if password != nil {
		log.Info("Accepting password", "password", *password, "len", len(*password))
		passwordLength = new(int64)
		*passwordLength = int64(len(*password))
		hasher := sha256.New()
		hasher.Write([]byte(*password))
		passwordSha256 = hasher.Sum(nil)
		passwordSha256Str = base64.StdEncoding.EncodeToString(passwordSha256)
		passwordHash = &passwordSha256Str
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
	var textKeyId *int64
	var hashKeyId *int64
	var ed25519PrivateKey ed25519.PrivateKey
	var ed25519PublicKey ed25519.PublicKey
	var privateKeyId *int64

	if publicKey == nil {
		log.Info("No public key provided, gathering one")
		if password == nil || passwordLength == nil || passwordHash == nil || passwordHashType == nil || passwordSha256 == nil {
			log.Error("No public key or password provided", "password", *password, "passwordLength", *passwordLength, "passwordHash", *passwordHash, "passwordHashType", *passwordHashType, "passwordSha256", passwordSha256)
			return false
		}

		if interactive != nil {
			publicKeyStr, err := auth.GetPublicKeyFromText(passwordSha256Str, "%")
			if err != nil {
				log.Info("Failed to get public key from text", "error", err)
			} else {
				log.Info("Got public key from text", "publicKeyStr", publicKeyStr)
				if publicKeyStr != "" {
					out, comment, options, rest, err := gossh.ParseAuthorizedKey([]byte(publicKeyStr))
					if err != nil {
						log.Error("Failed to parse public key", "error", err)
						return false
					}
					log.Info("Parsed public key", "out", out, "comment", comment, "options", options, "rest", rest)
					publicKey = &publicKeyStr
					publicKeyType = out.Type()

					key = out
					log.Info("Gathered public key", "publicKey", publicKeyStr)
					ctx.Permissions().Extensions["public-key-type"] = publicKeyType
					ctx.Permissions().Extensions["public-key-authorized"] = publicKeyStr
					log.Info("Setting permissions extensions", "public-key-type", publicKeyType, "public-key-authorized", publicKeyStr, "public-key", publicKeyStr, "public-key-type", publicKeyType)
				}
			}
		} else {
			publicKeyStr, err := auth.GetPublicKeyFromHash(passwordSha256Str, "%")
			if err != nil {
				log.Info("Failed to get public key from hash", "error", err)
			} else {
				log.Info("Got public key from hash", "publicKeyStr", publicKeyStr)
				if publicKeyStr != "" {
					out, comment, options, rest, err := gossh.ParseAuthorizedKey([]byte(publicKeyStr))
					if err != nil {
						log.Error("Failed to parse public key", "error", err)
						return false
					}
					log.Info("Parsed public key", "out", out, "comment", comment, "options", options, "rest", rest)
					publicKey = &publicKeyStr
					publicKeyType = out.Type()

					key = out
					log.Info("Gathered public key", "publicKey", publicKeyStr)
					ctx.Permissions().Extensions["public-key-type"] = publicKeyType
					ctx.Permissions().Extensions["public-key-authorized"] = publicKeyStr
					log.Info("Setting permissions extensions", "public-key-type", publicKeyType, "public-key-authorized", publicKeyStr, "public-key", publicKeyStr, "public-key-type", publicKeyType)
				}
			}
		}
		if key == nil {
			log.Info("No public key found, generating one")
			if interactive != nil {
				ed25519PrivateKey = ed25519.NewKeyFromSeed(passwordSha256)
				ed25519PublicKey = ed25519PrivateKey.Public().(ed25519.PublicKey)
			} else {
				var err error
				ed25519PublicKey, ed25519PrivateKey, err = ed25519.GenerateKey(nil)
				if err != nil {
					log.Error("Failed to generate private key", "error", err)
					return false
				}
			}
			log.Info("Generated private key", "pk", ed25519PrivateKey, "pkLen", len(ed25519PrivateKey), "pkStr", base64.StdEncoding.EncodeToString(ed25519PrivateKey))

			privateKeyIdi, err := auth.InsertPrivateKey(ed25519PrivateKey)
			if err != nil {
				log.Error("Failed to insert private key", "error", err)
				return false
			}
			privateKeyId = &privateKeyIdi

			log.Info("Generated public key", "pk", ed25519PublicKey, "pkLen", len(ed25519PublicKey), "pkStr", base64.StdEncoding.EncodeToString(ed25519PublicKey), "privateKeyId", *privateKeyId)
			ctx.Permissions().Extensions["private-key-seed"] = base64.StdEncoding.EncodeToString(ed25519PrivateKey.Seed())
			ctx.Permissions().Extensions["private-key"] = base64.StdEncoding.EncodeToString(ed25519PrivateKey)
			ctx.Permissions().Extensions["private-key-type"] = "ed25519"
			ctx.Permissions().Extensions["public-key"] = base64.StdEncoding.EncodeToString(ed25519PublicKey)
			ctx.Permissions().Extensions["public-key-type"] = "ed25519"

			sshPubKey, err := gossh.NewPublicKey(ed25519PublicKey)
			if err != nil {
				log.Fatal("Failed to create SSH public key", err)
			}

			if interactive != nil {
				textKeyIdi, err := auth.InsertTextPublicKey(passwordSha256Str, "sha256", sshPubKey)
				if err != nil {
					log.Error("Failed to insert text public key", "error", err)
					return false
				}
				textKeyId = &textKeyIdi
				log.Info("Inserted text public key", "textKeyId", *textKeyId)
			} else {
				hashKeyIdi, err := auth.InsertHashPublicKey(passwordSha256Str, "sha256", sshPubKey)
				if err != nil {
					log.Error("Failed to insert hash public key", "error", err)
					return false
				}
				hashKeyId = &hashKeyIdi
				log.Info("Inserted hash public key", "hashKeyId", *hashKeyId)
			}

			authorizedKey := gossh.MarshalAuthorizedKey(sshPubKey)
			authKey := string(authorizedKey)
			log.Info("Generated public key", "authKey", authKey, "authorizedKey", authorizedKey, "sshPubKey", sshPubKey, "sshPubKeyStr", string(sshPubKey.Marshal()))
			ctx.Permissions().Extensions["public-key-authorized"] = authKey

			publicKeyStr := base64.StdEncoding.EncodeToString(authorizedKey)
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
			pkMelted, err := melt.ToMnemonic(&ed25519PrivateKey)
			if err != nil {
				log.Error("Failed to melt private key", "error", err)
				return false
			}
			ctx.Permissions().Extensions["private-key-seed-melted"] = pkMelted
			log.Info("Melted private key", "pkMelted", pkMelted)
		}
	} else {
		log.Info("Public key provided", "publicKey", *publicKey, "key", key, "keyType", key.Type(), "keyMarshal", key.Marshal(), "keyMarshalLen", len(key.Marshal()))
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

	interactiveStr := ""
	if interactive != nil {
		interactiveStr = *interactive
	}

	connection := auth.Connection{
		Status:                     &status,
		Name:                       &user,
		Description:                &user,
		App:                        &app,
		AuthMethod:                 &authMethod,
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
		Interactive:                &interactiveStr,
		PasswordLength:             passwordLength,
		PasswordHash:               passwordHash,
		PasswordHashType:           passwordHashType,
		Admin:                      &admin,
		Query:                      &query,
		Commands:                   &commands,
		Comments:                   &comments,
		History:                    &history,
	}

	log.Info("Inserting connection", "connection", connection.ToData(), "connectionID", connection.ConnectionID)
	connectionID, err := connection.Insert()

	if err != nil {
		log.Error("Failed to insert connection", "error", err, "connectionID", connection.ConnectionID)
		return false
	}
	log.Info("Inserted connection", "connectionID", &connectionID, "connection", connection.String(), "connectionID", connection.ConnectionID)
	ctx.Permissions().Extensions["connection-id"] = fmt.Sprintf("%d", &connectionID)

	permissionsExtensionsJson, err := json.Marshal(ctx.Permissions().Extensions)
	if err != nil {
		log.Error("Failed to marshal extensions", "error", err, "connectionID", connection.ConnectionID)
		return false
	}
	log.Info("Setting permissions extensions", "permissionsExtensions", string(permissionsExtensionsJson), "connectionID", connection.ConnectionID)
	connection.SetPermissionsExtensions(string(permissionsExtensionsJson))

	log.Info("Checking public key", "publicKey", *publicKey, "connectionID", connection.ConnectionID)
	result, err := auth.CheckPublicKey(ctx, key)

	log.Info("Checked public key", "result", result, "error", err, "connectionID", connection.ConnectionID)
	if err != nil {
		var userID int64
		userID, err = auth.InsertUser(ctx)
		if err != nil {
			log.Error("Failed to insert user", "error", err, "connectionID", connection.ConnectionID)
			return false
		}
		log.Info("Inserted user", "userID", userID, "connectionID", connection.ConnectionID)

		var pk int64
		pk, err = auth.InsertPublicKey(userID, key)
		if err != nil {
			log.Error("Failed to insert public key", "error", err, "connectionID", connection.ConnectionID)
			return false
		}
		log.Info("Inserted public key", "pk", pk, "connectionID", connection.ConnectionID)

		result, err = auth.CheckPublicKey(ctx, key)

		log.Info("Checked public key", "result", result, "error", err, "connectionID", connection.ConnectionID)
	} else {
		log.Info("Public key already exists", "result", result, "connectionID", connection.ConnectionID)
	}
	if err != nil {
		log.Error("Failed to check public key", "error", err, "connectionID", connection.ConnectionID)
		return false
	}
	connection.SetCharmID(result.ID)
	if ed25519PrivateKey != nil {
		affected, err := auth.UpdatePrivateKey(*privateKeyId, &result.ID, connectionID)
		if err != nil {
			log.Error("Failed to update private key", "error", err, "connectionID", connection.ConnectionID)
			return false
		}
		log.Info("Updated private key", "affected", affected, "connectionID", connection.ConnectionID)
		if affected < 1 {
			log.Error("Failed to update private key, affected 0", "error", err, "connectionID", connection.ConnectionID)
			return false
		}
	}
	if textKeyId != nil {
		affected, err := auth.UpdateTextPublicKey(*textKeyId, &result.ID, connectionID)
		if err != nil {
			log.Error("Failed to update text public key", "error", err, "connectionID", connection.ConnectionID)
			return false
		}
		log.Info("Updated text public key", "affected", affected, "connectionID", connection.ConnectionID)

		if affected < 1 {
			log.Error("Failed to update text public key, affected 0", "error", err, "connectionID", connection.ConnectionID)
			return false
		}
	}
	if hashKeyId != nil {
		affected, err := auth.UpdateHashPublicKey(*hashKeyId, &result.ID, connectionID)
		if err != nil {
			log.Error("Failed to update hash public key", "error", err, "connectionID", connection.ConnectionID)
			return false
		}
		log.Info("Updated hash public key", "affected", affected, "connectionID", connection.ConnectionID)
		if affected < 1 {
			log.Error("Failed to update hash public key, affected 0", "error", err, "connectionID", connection.ConnectionID)
			return false
		}
	}
	ctx.Permissions().Extensions["charm-id"] = result.ID
	ctx.Permissions().Extensions["charm-name"] = result.Name
	log.Info("Setting permissions extensions", "charm-id", result.ID, "charm-name", result.Name, "connectionID", connection.ConnectionID)
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
