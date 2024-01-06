package main

import (
	"context"
	"encoding/base64"
	"errors"
	"fmt"
	"io"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/charmbracelet/log"
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
			func(h ssh.Handler) ssh.Handler {
				return func(s ssh.Session) {
					wish.Println(s, "Hello, world! 1")
					h(s)
					h(s)
					wish.Println(s, "Hello, world! 2")
				}
			},
			func(h ssh.Handler) ssh.Handler {
				return func(s ssh.Session) {
					if s.PublicKey() == nil {
						wish.Println(s, "Public key not required!")
						// x := s.Context()
						// x.Permissions().Extensions["password-hash"]
						// x.Permissions().Extensions["password-hash-type"]
					} else {
						authorizedKey := gossh.MarshalAuthorizedKey(s.PublicKey())
						io.WriteString(s, fmt.Sprintf("public key used by %s:\n", s.User()))
						s.Write(authorizedKey)
						pkid, err := auth.CheckPublicKey(s.Context(), s.PublicKey())
						switch {
						case err == nil:
							wish.Println(s, "Hey!", pkid)

						default:
							publicKeyType := s.PublicKey().Type()
							publicKeyData := base64.StdEncoding.EncodeToString(s.PublicKey().Marshal())
							message := fmt.Sprintf("Hey, I don't know who you are!\nError:\n%v\nPublic key:\n%s %s", err, publicKeyType, publicKeyData)
							wish.Println(s, message)

							uid, err := auth.InsertUser(s.Context())
							if err != nil {
								wish.Println(s, "Failed to insert user:", err)
							} else {
								wish.Println(s, "Inserted user id:", uid)

								pkid, err := auth.InsertPublicKey(uid, s.PublicKey())
								if err != nil {
									wish.Println(s, "Failed to insert public key:", err)
								} else {
									wish.Println(s, "Inserted public key id: ", pkid, fmt.Sprintf("\n%s\n%s %s", "Inserted public key:", publicKeyType, publicKeyData))
								}
							}
						}
					}
					h(s)
				}
			},
			func(h ssh.Handler) ssh.Handler {
				return func(s ssh.Session) {
					wish.Println(s, "Goodbye, world! 1")
					h(s)
					wish.Println(s, "Goodbye, world! 2")
				}
			},
			scp.Middleware(handler, handler),
			comment.Middleware("Thanks, have a nice day!"),
			elapsed.Middleware(),
			promwish.Middleware("0.0.0.0:9222", "identity"),
			logging.Middleware(),
		),
		// wish.WithPasswordAuth(func(ctx ssh.Context, password string) bool {
		// 	log.Info("Accepting password", "password", password, "len", len(password))

		// 	passwordSha := base64.StdEncoding.EncodeToString(sha256.New().Sum([]byte(password)))
		// 	if ctx.Permissions().Extensions == nil {
		// 		ctx.Permissions().Extensions = make(map[string]string)
		// 	}
		// 	ctx.Permissions().Extensions["password-hash"] = passwordSha
		// 	ctx.Permissions().Extensions["password-hash-type"] = "sha256"

		// 	log.Info("Accepting password", "passwordSha", passwordSha)

		// 	return true
		// }),
		wish.WithKeyboardInteractiveAuth(func(ctx ssh.Context, challenge gossh.KeyboardInteractiveChallenge) bool {
			log.Info("Accepting keyboard interactive", "challenge", challenge)
			response, err := challenge("", "", []string{"Room:"}, []bool{true})
			if err != nil {
				log.Error("Failed to get keyboard interactive response", "error", err)
				return false
			}
			log.Info("Accepting keyboard interactive", "response", response, "room", response[0])
			return true
		}),
		wish.WithPublicKeyAuth(func(ctx ssh.Context, key ssh.PublicKey) bool {
			log.Info("Accepting public key", "publicKeyType", key.Type(), "publicKeyString", base64.StdEncoding.EncodeToString(key.Marshal()))
			return true
		}),
		// TODO: use the function to include per-user information and to check database
		wish.WithBanner(`
Welcome to the identity server!

By using this service, you agree to the following terms and conditions:

- EACH PARTY MAKES NO WARRANTIES, EXPRESS, IMPLIED OR OTHERWISE, REGARDING ITS ACCURACY, COMPLETENESS OR PERFORMANCE.

- THE SERVICE AND ANY RELATED SERVICES ARE PROVIDED ON AN "AS IS" AND "AS AVAILABLE" BASIS, WITHOUT WARRANTY OF ANY KIND, WHETHER WRITTEN OR ORAL, EXPRESS OR IMPLIED.

- TO THE FULL EXTENT PERMISSIBLE BY LAW, DEVELOPING.TODAY LLC WILL NOT BE LIABLE FOR ANY DAMAGES OF ANY KIND ARISING FROM THE USE OF ANY DEVELOPING.TODAY LLC SERVICE, OR FROM ANY INFORMATION, CONTENT, MATERIALS, PRODUCTS (INCLUDING SOFTWARE) OR OTHER SERVICES INCLUDED ON OR OTHERWISE MADE AVAILABLE TO YOU THROUGH ANY DEVELOPING.TODAY LLC SERVICE, INCLUDING, BUT NOT LIMITED TO DIRECT, INDIRECT, INCIDENTAL, PUNITIVE, AND CONSEQUENTIAL DAMAGES, UNLESS OTHERWISE SPECIFIED IN WRITING.

If you do not agree to these terms and conditions, you may not use this service and must disconnect immediately.

`+fmt.Sprintf("You are using the identity server at %s:%d\n", configuration.String("host"), configuration.Int("port"))),
		wish.WithAddress(fmt.Sprintf("%s:%d", configuration.String("host"), configuration.Int("port"))),
		wish.WithHostKeyPath(".ssh/term_info_ed25519"),
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
