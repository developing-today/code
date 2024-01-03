package main

import (
	"context"
	"encoding/base64"
	"errors"
	"fmt"
	"io"
	"os"
	"os/signal"
	"strings"
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
	koanf "github.com/knadh/koanf"
	kpakdl "github.com/knadh/koanf/parsers/kdl"
	kprfile "github.com/knadh/koanf/providers/file"
	kdl "github.com/sblinch/kdl-go"
	"github.com/spf13/cobra"
	gossh "golang.org/x/crypto/ssh"
)

func initConfig() {
	koanf := koanf.New(".")
	koanf.Load(kprfile.Provider("./config.kdl"), kpakdl.Parser())
	log.Info("Loaded config", "config", koanf.Sprint())
}

func init() {
	cobra.OnInitialize(initConfig)
	rootCmd.AddCommand(startCmd)
}

const (
	host = "0.0.0.0"
	port = 42
)

var rootCmd = &cobra.Command{
	Use:   "yourAppName",
	Short: "Your application's short description",
	Long:  `A longer description...`,
}

var startCmd = &cobra.Command{
	Use:   "start",
	Short: "Starts the SSH server",
	Run:   startSSHServer,
}

func main() {
	if err := rootCmd.Execute(); err != nil {
		fmt.Println(err)
		os.Exit(1)
	}
}

func startSSHServer(cmd *cobra.Command, args []string) {
	data := `
    (first)name "Bob"
    age 76
    active true
`

	if doc, err := kdl.Parse(strings.NewReader(data)); err == nil {
		log.Info("Parsed KDL document", "document", doc, "len(doc.Nodes)", len(doc.Nodes), "doc.Nodes[0].Name", doc.Nodes[0].Name)
	}
	handler := scp.NewFileSystemHandler("./files")

	s, err := wish.NewServer(
		wish.WithAddress(fmt.Sprintf("%s:%d", host, port)),
		wish.WithHostKeyPath(".ssh/term_info_ed25519"),
		wish.WithPublicKeyAuth(func(ctx ssh.Context, key ssh.PublicKey) bool {
			log.Info("Accepting public key", "publicKeyType", key.Type(), "publicKeyString", base64.StdEncoding.EncodeToString(key.Marshal()))
			return true
		}),
		wish.WithMiddleware(
			logging.Middleware(),
			func(h ssh.Handler) ssh.Handler {
				return func(s ssh.Session) {
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
					h(s)
				}
			},
			comment.Middleware("Thanks, have a nice day!"),
			promwish.Middleware("0.0.0.0:9222", "identity"),
			scp.Middleware(handler, handler),
			elapsed.Middleware(),
		),
	)
	if err != nil {
		log.Error("could not start server", "error", err)
		return
	}

	done := make(chan os.Signal, 1)
	signal.Notify(done, os.Interrupt, syscall.SIGINT, syscall.SIGTERM)
	log.Info("Starting SSH server", "host", host, "port", port)
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
