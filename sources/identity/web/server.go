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
	"github.com/knadh/koanf"

	gowebly "github.com/gowebly/helpers"
)

//go:embed all:static
var static embed.FS

// todo make a input struct for webserver and use opts pattern

func RunWebServer(connections *auth.SafeConnectionMap, configuration *koanf.Koanf) {
	if err := RunServer(connections, configuration); err != nil {
		log.Error("Failed to start server!", "details", err.Error())
		os.Exit(1)
	}
}

func GoRunWebServer(connections *auth.SafeConnectionMap, configuration *koanf.Koanf) {
	go RunWebServer(connections, configuration)
}

// runServer runs a new HTTP server with the loaded environment variables.
func RunServer(connections *auth.SafeConnectionMap, configuration *koanf.Koanf) error {

	// jwtMiddleware, err := setupJWTMiddleware(configuration)
	// if err != nil {
	// 	log.Error("Failed to set up JWT middleware!", "details", err.Error())
	// 	return err
	// }

	port, err := strconv.Atoi(gowebly.Getenv("BACKEND_PORT", "7000"))
	if err != nil {
		return err
	}
	router := chi.NewRouter()
	router.Use(ConnectionsMiddleware(connections))
	router.Use(middleware.Logger)
	router.Handle("/static/*", gowebly.StaticFileServerHandler(http.FS(static)))
	router.Get("/admin/connections", indexViewHandler)
	// router.With(jwtMiddleware.CheckJWT).Get("/admin/api/id", showIDAPIHandler)
	router.Post("/admin/api/id", showIDAPIHandler)

	router.Get("/set-cookie", setCookieHandler)
	router.Get("/invalidate-cookie", invalidateCookieHandler)

	server := &http.Server{
		Addr:         fmt.Sprintf(":%d", port),
		Handler:      router, // handle all chi routes
		ReadTimeout:  10 * time.Second,
		WriteTimeout: 20 * time.Second,
	}

	log.Info("Starting web server...", "port", port)

	return server.ListenAndServe()
}

func invalidateCookieHandler(w http.ResponseWriter, r *http.Request) {
	cookie, err := r.Cookie("token")

	if err != nil {
		w.Write([]byte("No cookie found"))
		return
	}
	log.Printf("Cookie found: %s", cookie.Value)
	// expire the cookie in the database

	emptyCookie := &http.Cookie{
		Name:     "token",
		Value:    "",
		Expires:  time.Unix(0, 0),
		HttpOnly: true,
		MaxAge:   -1,
	}

	http.SetCookie(w, emptyCookie)
	w.Write([]byte("Cookie invalidated"))
}

func setCookieHandler(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Error(w, "Method not allowed", http.StatusMethodNotAllowed)
		return
	}
	// validate existing token cookie, do nothing if valid (validate checks cache, if not found, checks db)

	token := r.FormValue("token")
	log.Printf("Received token: %s", token)

	// check for existing token cookie in cache, assign that if found

	// validate token, if valid, set cookie in cache and db

	w.WriteHeader(http.StatusNoContent)

	http.SetCookie(w, &http.Cookie{
		Name:     "token",
		Value:    "validationToken",
		Expires:  time.Now().Add(24 * time.Hour),
		HttpOnly: true,
	})
}

func ConnectionsMiddleware(connections *auth.SafeConnectionMap) func(next http.Handler) http.Handler {
	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			ctx := auth.WithConnectionMap(r.Context(), connections)
			next.ServeHTTP(w, r.WithContext(ctx))
		})
	}
}

func jwtValidationMiddleware(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		tokenString := r.Header.Get("Authorization")
		// tokenString := extractToken(r) // Implement this function to extract the JWT from the request
		if tokenString == "" {
			http.Error(w, "Unauthorized", http.StatusUnauthorized)
			return
		}

		// Use the previously shown validateJWT function
		// valid, err := validateJWT(tokenString, "https://yourdomain.com/.well-known/jwks.json")
		// if err != nil || !valid {
		// 	http.Error(w, "Unauthorized", http.StatusUnauthorized)
		// 	return
		// }

		// Token is valid; proceed with the request
		next.ServeHTTP(w, r)
	})
}

// func setupJWTMiddleware(configuration *koanf.Koanf) (*jwtmiddleware.JWTMiddleware, error) {
// 	issuer := configuration.String("identity.server.jwt.issuer")
// 	audience := []string{configuration.String("identity.server.jwt.audience")}

// 	issuerURL, err := url.Parse(issuer)
// 	if err != nil {
// 		return nil, fmt.Errorf("failed to parse the issuer URL: %v", err)
// 	}

// 	provider := jwks.NewCachingProvider(issuerURL, 15*time.Minute)

// 	jwtValidator, err := validator.New(
// 		provider.KeyFunc,
// 		// ecdsa,
// 		validator.
// 			issuerURL.String(),
// 		audience,
// 	)
// 	if err != nil {
// 		log.Error("Failed to set up the validator!", "details", err.Error())
// 		return nil, fmt.Errorf("failed to set up the validator: %v", err)
// 	}
// 	log.Info("JWT middleware set up successfully")

// 	return jwtmiddleware.New(jwtValidator.ValidateToken), nil
// }
