package web

import (
	"embed"
	"fmt"
	"net/http"
	"os"
	"strconv"
	"time"

	"github.com/charmbracelet/log"
	"github.com/developing-today/code/src/identity/auth"
	"github.com/go-chi/chi/v5"
	"github.com/go-chi/chi/v5/middleware"

	gowebly "github.com/gowebly/helpers"
)

//go:embed all:static
var static embed.FS

func RunWebServer(connections *auth.SafeConnectionMap) {
	if err := RunServer(connections); err != nil {
		log.Error("Failed to start server!", "details", err.Error())
		os.Exit(1)
	}
}

func GoRunWebServer(connections *auth.SafeConnectionMap) {
	go RunWebServer(connections)
}

// runServer runs a new HTTP server with the loaded environment variables.
func RunServer(connections *auth.SafeConnectionMap) error {
	// Validate environment variables.
	port, err := strconv.Atoi(gowebly.Getenv("BACKEND_PORT", "7000"))
	if err != nil {
		return err
	}

	// Create a new chi router.
	router := chi.NewRouter()

	router.Use(ConnectionsMiddleware(connections))
	// Use chi middlewares.
	router.Use(middleware.Logger)

	// Handle static files from the embed FS (with a custom handler).
	router.Handle("/static/*", gowebly.StaticFileServerHandler(http.FS(static)))

	// Handle index page view.
	router.Get("/", indexViewHandler)

	// Handle API endpoints.
	router.Get("/api/hello-world", showContentAPIHandler)
	router.Get("/api/id", showIDAPIHandler)

	// Create a new server instance with options from environment variables.
	// For more information, see https://blog.cloudflare.com/the-complete-guide-to-golang-net-http-timeouts/
	server := &http.Server{
		Addr:         fmt.Sprintf(":%d", port),
		Handler:      router, // handle all chi routes
		ReadTimeout:  5 * time.Second,
		WriteTimeout: 10 * time.Second,
	}

	// Send log message.
	log.Info("Starting web server...", "port", port)

	return server.ListenAndServe()
}

func ConnectionsMiddleware(connections *auth.SafeConnectionMap) func(next http.Handler) http.Handler {
	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			// Insert the entire connections map into the context
			ctx := auth.WithConnectionMap(r.Context(), connections)
			// Call the next handler with the new context
			next.ServeHTTP(w, r.WithContext(ctx))
		})
	}
}
