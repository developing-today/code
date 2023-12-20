package main

import (
	"context"
	"encoding/base64"
	"errors"
	"os/signal"
	"syscall"
	"time"

	"fmt"
	"os"

	"github.com/charmbracelet/log"
	"github.com/charmbracelet/ssh"
	"github.com/charmbracelet/wish"
	"github.com/charmbracelet/wish/logging"
	"github.com/developing-today/code/src/identity/auth"
)

const (
	host = "localhost"
	port = 23234
)

func main() {
	s, err := wish.NewServer(
		wish.WithAddress(fmt.Sprintf("%s:%d", host, port)),
		wish.WithHostKeyPath(".ssh/term_info_ed25519"),
		wish.WithPublicKeyAuth(func(ctx ssh.Context, key ssh.PublicKey) bool {
			return true
		}),
		wish.WithMiddleware(
			logging.Middleware(),
			func(h ssh.Handler) ssh.Handler {
				return func(s ssh.Session) {
					result, err := auth.CheckPublicKey(s.Context(), s.PublicKey())
					switch {
					case err == nil:
						wish.Println(s, "Hey!", result)

					default:
						publicKeyType := s.PublicKey().Type()
						publicKeyData := base64.StdEncoding.EncodeToString(s.PublicKey().Marshal())
						message := fmt.Sprintf("Hey, I don't know who you are! Error: %v, Result: %v, Public Key Type: %s, Public Key: %s", err, result, publicKeyType, publicKeyData)
						wish.Println(s, message)
					}
					h(s)
				}
			},
		),
	)
	if err != nil {
		log.Error("could not start server", "error", err)
	}

	done := make(chan os.Signal, 1)
	signal.Notify(done, os.Interrupt, syscall.SIGINT, syscall.SIGTERM)
	log.Info("Starting SSH server", "host", host, "port", port)
	go func() {
		if err = s.ListenAndServe(); err != nil && !errors.Is(err, ssh.ErrServerClosed) {
			log.Error("could not start server", "error", err)
			done <- nil
		}
	}()

	<-done
	log.Info("Stopping SSH server")
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer func() { cancel() }()
	if err := s.Shutdown(ctx); err != nil && !errors.Is(err, ssh.ErrServerClosed) {
		log.Error("could not stop server", "error", err)
	}
}
