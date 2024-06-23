package stream

import (
	"context"
	"net/http"
	"time"

	"github.com/centrifugal/centrifuge"
	"github.com/charmbracelet/log"
	"github.com/spf13/cobra"
)

func RunStreamServer(ctx context.Context, cmd *cobra.Command, args []string) {
	log.Info("Starting stream server")
	node, err := centrifuge.New(centrifuge.Config{})
	if err != nil {
		log.Fatal(err)
	}

	SetupNodeHandlers(node)

	srv := &http.Server{Addr: "0.0.0.0:8001", Handler: nil}

	wsHandler := centrifuge.NewWebsocketHandler(node, centrifuge.WebsocketConfig{})
	http.Handle("/connection/websocket", Auth(wsHandler))
	http.Handle("/", http.FileServer(http.Dir("./")))

	go func() {
		<-ctx.Done()

		shutdownCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()

		log.Info("Shutting down server")
		if err := srv.Shutdown(shutdownCtx); err != nil {
			log.Error("Shutdown error", "error", err)
		}
	}()

	log.Info("Starting server", "url", srv.Addr)
	if err := srv.ListenAndServe(); err != http.ErrServerClosed {
		log.Fatal("ListenAndServe error", "error", err)
	}
}

func SetupNodeHandlers(node *centrifuge.Node) {
	log.Info("Setting up node handlers")
	node.OnConnect(func(client *centrifuge.Client) {
		transportName := client.Transport().Name()
		transportProto := client.Transport().Protocol()
		log.Info("Client connected", "transportName", transportName, "transportProto", transportProto)

		client.OnSubscribe(func(e centrifuge.SubscribeEvent, cb centrifuge.SubscribeCallback) {
			log.Info("Client subscribes on channel", "channel", e.Channel)
			cb(centrifuge.SubscribeReply{}, nil)
		})

		client.OnPublish(func(e centrifuge.PublishEvent, cb centrifuge.PublishCallback) {
			log.Info("Client publishes into channel", "channel", e.Channel, "data", string(e.Data))
			if cb == nil {
				log.Info("Publish Callback is null")
			} else {
				cb(centrifuge.PublishReply{}, nil)
			}
		})

		client.OnDisconnect(func(e centrifuge.DisconnectEvent) {
			log.Info("Client disconnected")
		})

		log.Info("Client handlers set up")
	})
}

func Auth(h http.Handler) http.Handler {
	log.Info("Setting up auth middleware")
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		ctx := r.Context()
		cred := &centrifuge.Credentials{
			UserID: "",
		}
		newCtx := centrifuge.SetCredentials(ctx, cred)
		r = r.WithContext(newCtx)
		log.Info("Authenticating request", "url", r.URL.String())
		h.ServeHTTP(w, r)
	})
}
