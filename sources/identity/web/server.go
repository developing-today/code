package web

import (
	"context"
	"embed"
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"strings"
	"time"

	"crypto/ed25519"

	"github.com/auth0/go-jwt-middleware/v2/jwks"
	"github.com/charmbracelet/log"
	"github.com/developing-today/code/src/identity/auth"
	"github.com/developing-today/code/src/identity/configuration" // replace with http ?
	"github.com/go-chi/chi"
	"github.com/go-chi/chi/middleware"

	// ???
	"github.com/golang-jwt/jwt"
	"github.com/knadh/koanf"
	"gopkg.in/go-jose/go-jose.v2"

	gowebly "github.com/gowebly/helpers"
)

//go:embed all:static
var static embed.FS

// todo make a input struct for webserver and use opts pattern

func RunWebServer(ctx context.Context, connections *auth.SafeConnectionMap, config *koanf.Koanf) {
	if err := RunServer(ctx, connections, config); err != nil {
		log.Error("Failed to start server!", "details", err.Error())
		os.Exit(1)
	}
}

func GoRunWebServer(ctx context.Context, connections *auth.SafeConnectionMap, configuration *configuration.IdentityServerConfiguration) {
	go RunWebServer(ctx, connections, configuration.Configuration)
}

func Strings(config *koanf.Koanf, key string) []string {
	stringArray := config.Strings(key)
	if len(stringArray) > 0 {
		return stringArray
	}
	stringValue := config.String(key)

	if stringValue == "" {
		log.Error("Empty string value", "key", key)
		return []string{}
	}

	return []string{stringValue}
}

func RunServer(ctx context.Context, connections *auth.SafeConnectionMap, config *koanf.Koanf) error {
	jwksURLString := config.String("identity.server.jwt.jwks")
	log.Info("JWKS URL", "jwks", jwksURLString)
	jwksURL, err := url.Parse(jwksURLString)
	if err != nil {
		return fmt.Errorf("failed to parse JWKS URL: %v", err)
	}
	log.Info("JWKS URL", "jwks", jwksURL)
	issuer := config.String("identity.server.jwt.issuer")
	log.Info("Issuer", "issuer", issuer)
	issuerURL, err := url.Parse(issuer)
	if err != nil {
		return fmt.Errorf("failed to parse issuer URL: %v", err)
	}
	log.Info("Issuer URL", "issuer", issuerURL)
	audience := Strings(config, "identity.server.jwt.audience")
	log.Info("Audience List", "audience", audience)
	if len(audience) == 0 {
		return fmt.Errorf("audience list is empty")
	}
	cacheTTL := config.Duration("identity.server.jwt.cache_ttl")
	log.Info("Cache TTL from config", "cacheTTL", cacheTTL)
	if cacheTTL == 0 {
		cacheTTL = 15 * time.Minute
	}
	log.Info("Cache TTL", "cacheTTL", cacheTTL)

	keyFunc, err := NewJWKSValidator(jwksURL, issuerURL, audience, cacheTTL)
	if err != nil {
		log.Error("Failed to create JWKS validator", "error", err)
		return err
	}

	webPort := config.Int("identity.server.web.port")
	log.Info("Web Port", "port", webPort)
	if webPort == 0 {
		webPort = 7000
		log.Info("Using default Web Port", "port", webPort)
	}

	authorizationCookieName := config.String("identity.server.authorization.cookie_name")
	log.Info("Cookie Name", "cookieName", authorizationCookieName)

	if authorizationCookieName == "" {
		authorizationCookieName = "Authorization"
		log.Info("Using default Cookie Name", "cookieName", authorizationCookieName)
	}
	authorizationHeaderName := config.String("identity.server.authorization.header_name")
	log.Info("Header Name", "headerName", authorizationHeaderName)
	if authorizationHeaderName == "" {
		authorizationCookieName = "Authorization"
		log.Info("Using default Header Name", "headerName", authorizationHeaderName)
	}
	authorizationHeaderPrefix := config.String("identity.server.authorization.header_prefix")
	log.Info("Header Prefix", "headerPrefix", authorizationHeaderPrefix)
	if authorizationHeaderPrefix == "" {
		authorizationHeaderPrefix = "Bearer"
		log.Info("Using default Header Prefix", "headerPrefix", authorizationHeaderPrefix)
	}

	router := chi.NewRouter() // todo try http.NewServeMux if they add a use, with, route
	router.Use(ConnectionsMiddleware(connections))
	router.Use(middleware.Logger)
	router.Handle("/static/*", gowebly.StaticFileServerHandler(http.FS(static)))
	router.Get("/admin/connections", indexViewHandler)
	router.With(
		CookieMiddleware(authorizationCookieName, keyFunc),
	).Post("/admin/api/id", showIDAPIHandler)
	router.With(
		BearerMiddleware(keyFunc, authorizationHeaderName, authorizationHeaderPrefix),
	).Post(
		"/admin/rest/id",
		showIDAPIJsonHandler)
	router.Post("/set-cookie", setCookieHandler(keyFunc, authorizationCookieName))
	router.Get("/invalidate-cookie", invalidateCookieHandler(authorizationCookieName))

	server := &http.Server{
		Addr:         fmt.Sprintf(":%d", webPort),
		Handler:      router,
		ReadTimeout:  10 * time.Second,
		WriteTimeout: 20 * time.Second,
	}

	go func() {
		log.Info("Starting web server", "port", webPort)
		if err := server.ListenAndServe(); err != http.ErrServerClosed {
			log.Error("HTTP server ListenAndServe error:", "error", err)
		}
	}()

	log.Info("Web server started", "port", webPort)

	<-ctx.Done()

	shutdownCtx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	log.Info("Shutting down web server...")

	if err := server.Shutdown(shutdownCtx); err != nil {
		log.Error("Server shutdown error:", "error", err)
		return err
	}

	log.Info("Web server stopped")

	return nil
}

func NewJWKSValidator(jwksURL *url.URL, issuer *url.URL, audience []string, cacheTTL time.Duration) (jwt.Keyfunc, error) {
	jwksClient := jwks.NewCachingProvider(issuer, cacheTTL, jwks.WithCustomJWKSURI(jwksURL)) // probably find a way to remove custom jwks
	log.Info("JWKS Validator", "jwksClientCacheTTL", cacheTTL, "jwksURL", jwksURL, "issuer", issuer, "audience", audience, "jwksClientCustomJWKSURI", jwksClient.CustomJWKSURI, "jwksClientIssuerURL", jwksClient.IssuerURL)

	return func(token *jwt.Token) (any, error) {
		log.Info("Validating token", "token", token)

		alg, ok := token.Header["alg"].(string)
		if !ok {
			log.Error("Expecting JWT header to have string 'alg'")
			return nil, fmt.Errorf("expecting JWT header to have string 'alg'")
		}
		log.Info("Algorithm", "alg", alg)
		if alg == "" || strings.ToLower(alg) == "none" {
			log.Error("Invalid algorithm", "alg", alg)
			return nil, fmt.Errorf("invalid algorithm")
		}

		kid, ok := token.Header["kid"].(string)
		if !ok {
			log.Error("Expecting JWT header to have string 'kid'")
			return nil, fmt.Errorf("expecting JWT header to have string 'kid'")
		}
		log.Info("Key ID", "kid", kid)

		cacheSetInterface, err := jwksClient.KeyFunc(context.Background())
		if err != nil {
			log.Error("Failed to get JWKS from cache", "error", err)
			return nil, fmt.Errorf("failed to get JWKS from cache: %w", err)
		}
		log.Info("Cache Set", "cacheSet", cacheSetInterface, "type", fmt.Sprintf("%T", cacheSetInterface), "convertedCacheSet", cacheSetInterface.(*jose.JSONWebKeySet))

		cacheSet, ok := cacheSetInterface.(*jose.JSONWebKeySet)
		if !ok {
			log.Error("Failed to convert cache set to *jose.JSONWebKeySet")
			return nil, fmt.Errorf("failed to convert cache set to *jose.JSONWebKeySet")
		}
		log.Info("Cache Set", "cacheSet", cacheSet)

		cacheKeys := cacheSet.Key(kid)

		if len(cacheKeys) == 0 {
			log.Error("Key not found in cache", "kid", kid)
			return nil, fmt.Errorf("key %v not found in cache", kid)
		}

		log.Info("Cache Keys", "cacheKeys", cacheKeys)

		cacheKey := cacheKeys[0]
		log.Info("Cache Key", "cacheKey", cacheKey)

		if cacheKey.KeyID != kid { // todo allow multiple keys
			log.Error("Key ID does not match", "kid", kid, "keyID", cacheKey.KeyID, "cacheKey", cacheKey)
			return nil, fmt.Errorf("key ID does not match")
		}
		if cacheKey.Algorithm != alg {
			log.Error("Algorithm does not match", "alg", alg, "algorithm", cacheKey.Algorithm, "cacheKey", cacheKey)
			return nil, fmt.Errorf("algorithm does not match")
		}
		if !cacheKey.Valid() {
			log.Error("Key is not valid", "cacheKey", cacheKey)
			return nil, fmt.Errorf("key is not valid")
		}

		aud := token.Claims.(jwt.MapClaims)["aud"]
		log.Info("Audience", "aud", aud, "audience", audience)

		if !audienceContainsAll(aud.([]any), audience) {
			log.Error("Invalid audience", "aud", aud, "audience", audience)
			return nil, fmt.Errorf("invalid audience, expected %v, got %v", audience, aud)
		}

		switch key := cacheKey.Key.(type) {
		case ed25519.PublicKey:
			log.Info("Key", "key", key, "type", "ed25519.PublicKey")

			return key, nil
		default:
			log.Error("Key is not a valid type", "key", key, "type", fmt.Sprintf("%T", key), "expectedType", []string{"ed25519.PublicKey"})
			return nil, fmt.Errorf("key is not a valid type, expected %v, got %v", []string{"ed25519.PublicKey"}, fmt.Sprintf("%T", key))
		}
	}, nil
}

func audienceContains(aud string, audience []any) bool {
	for _, a := range audience {
		if a == aud {
			log.Info("Audience contains", "aud", a, "audience", audience)
			return true
		}
	}
	log.Error("Audience does not contain", "aud", aud, "audience", audience)
	return false
}

func audienceContainsAll(aud []any, audience []string) bool {
	for _, a := range audience {
		if !audienceContains(a, aud) {
			log.Error("Audience does not contain", "aud", a, "audience", aud)
			return false
		}
	}
	log.Info("Audience contains all", "audience", aud, "expectedAudience", audience)
	return true
}

func ExpireCookie(token string) error {
	// expire the cookie & close the connection in the database
	// close the connection in the cache
	return nil
}

func invalidateCookieHandler(cookieName string) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		cookie, err := r.Cookie(cookieName)

		if err != nil {
			w.Write([]byte("No cookie found"))
			return
		}
		log.Info("Cookie found", "cookie", cookie)

		err = ExpireCookie(cookie.Value)
		if err != nil {
			w.WriteHeader(http.StatusInternalServerError)
			return
		}

		emptyCookie := &http.Cookie{
			Name:     cookieName,
			Value:    "",
			Expires:  time.Unix(0, 0),
			HttpOnly: true,
			MaxAge:   -1,
		}

		http.SetCookie(w, emptyCookie)
		w.Write([]byte("Cookie invalidated"))
	}
}
func validateCookie(cookieName string) func(r *http.Request) (bool, error) {
	return func(r *http.Request) (bool, error) {
		cookie, err := r.Cookie(cookieName)
		if err != nil {
			return false, err
		}
		log.Info("Cookie found", "cookie", cookie)
		// validate the cookie in the cache, else the database, if found, return true, else false and error (is it found but expired?)

		return true, nil
	}
}

func parseToken(token string, keyFunc jwt.Keyfunc) (bool, error) {
	// validate the token in the cache, else the database, if found, return true, else false and error (is it found but expired?)
	if token == "" {
		log.Error("No token found")
		return false, nil
	}
	token = strings.Replace(token, "Bearer ", "", 1)
	token = strings.TrimSpace(token)
	token = strings.Trim(token, "'\"")

	parsed, err := jwt.Parse(token, keyFunc)

	if err != nil {
		log.Error("Error parsing token", "error", err)
		jsonToken, err := json.Marshal(token)
		if err != nil {
			log.Error("Error parsing token", "error", err)
		}
		log.Error("Token: ", "token", string(jsonToken))
		return false, err
	}

	logJWT(parsed)

	if !parsed.Valid {
		return false, nil
	}

	claims, ok := parsed.Claims.(jwt.MapClaims)

	if !ok {
		return false, fmt.Errorf("invalid claims")
	}

	log.Info("Claims", "claims", claims)

	return true, nil
}

func NewTokenConnection(token string, keyFunc jwt.Keyfunc) (*auth.Connection, error) {
	// validate the token in the cache, else the database, if found, return true, else false and error (is it found but expired?)
	if token == "" {
		return nil, nil
	}

	parsed, err := jwt.Parse(token, keyFunc)

	logJWT(parsed)

	if err != nil {
		log.Error("Error parsing token", "error", err)
		return nil, err
	}

	claims, ok := parsed.Claims.(jwt.MapClaims)

	if !ok {
		log.Error("Invalid claims")
		return nil, fmt.Errorf("invalid claims")
	}

	log.Info("Claims", "claims", claims)

	connection := &auth.Connection{}

	connection.Insert()

	log.Info("Connection created", "connection", connection)

	return connection, nil
}

func setCookieHandler(keyFunc jwt.Keyfunc, authorizationCookieName string) func(w http.ResponseWriter, r *http.Request) {
	return func(w http.ResponseWriter, r *http.Request) {

		// todo check current cookie first, if it exists, return error

		log.Info("Request", "request", r)
		token := r.FormValue(authorizationCookieName)
		if token == "" {
			http.Error(w, "Unauthorized", http.StatusUnauthorized)
			log.Error("No token found")
			return
		}
		log.Info("Token", "token", token)

		validToken, err := parseToken(token, keyFunc)
		if err != nil {
			http.Error(w, "Internal Server Error", http.StatusInternalServerError)
			log.Error("Error parsing token", "error", err)
			return
		}
		log.Info("Valid token", "validToken", validToken)

		connection, err := NewTokenConnection(token, keyFunc)
		if err != nil {
			http.Error(w, "Internal Server Error", http.StatusInternalServerError)
			log.Error("Error creating connection", "error", err)
			return
		}

		if connection == nil {
			http.Error(w, "Unauthorized", http.StatusUnauthorized)
			log.Error("No connection found")
			return
		}
		log.Info("Connection created", "connection", connection)

		cookie := &http.Cookie{
			Name:     authorizationCookieName,
			Value:    token,
			Expires:  time.Now().Add(24 * time.Hour),
			HttpOnly: true,
		}

		http.SetCookie(w, cookie)
		w.Write([]byte("Cookie set"))
		log.Info("Cookie set", "token", token, "cookie", cookie, "connection", connection)
	}
}

// BearerMiddleware extracts the JWT token from the Authorization header and delegates to JWTMiddleware.
func BearerMiddleware(keyFunc jwt.Keyfunc, authorizationHeaderName string, authorizationHeaderPrefix string) func(http.Handler) http.Handler {
	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			authHeader := r.Header.Get(authorizationHeaderName)
			if authHeader == "" {
				http.Error(w, "Authorization header is required", http.StatusUnauthorized)
				return
			}

			parts := strings.Split(authHeader, " ")
			if len(parts) != 2 || strings.ToLower(parts[0]) != strings.ToLower(authorizationHeaderPrefix) {
				http.Error(w, "Authorization header format is invalid", http.StatusUnauthorized)
				return
			}

			token := parts[1]
			JWTMiddleware(token, keyFunc)(next).ServeHTTP(w, r)
		})
	}
}

// CookieMiddleware extracts the JWT token from a specified cookie and delegates to JWTMiddleware.
func CookieMiddleware(cookieName string, keyFunc jwt.Keyfunc) func(http.Handler) http.Handler {
	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			cookie, err := r.Cookie(cookieName)
			if err != nil {
				http.Error(w, "Cookie is required", http.StatusUnauthorized)
				return
			}

			token := cookie.Value
			JWTMiddleware(token, keyFunc)(next).ServeHTTP(w, r)
		})
	}
}

// JWTMiddleware performs JWT validation and logs the token details.
func JWTMiddleware(token string, keyFunc jwt.Keyfunc) func(http.Handler) http.Handler {
	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			token, err := jwt.Parse(token, keyFunc)
			if err != nil {
				http.Error(w, "Invalid token", http.StatusUnauthorized)
				return
			}
			if !token.Valid {
				http.Error(w, "Invalid token", http.StatusUnauthorized)
				return
			}

			logJWT(token)

			ctx := context.WithValue(r.Context(), "claims", token.Claims)
			next.ServeHTTP(w, r.WithContext(ctx))
		})
	}
}

// logJWT logs the JWT header, payload (claims), and signature.
func logJWT(token *jwt.Token) {
	headerJson, err := json.Marshal(token.Header)
	if err != nil {
		log.Error("Failed to parse JWT header")
		return
	}
	log.Info("", "JWT Header", string(headerJson))

	claims, ok := token.Claims.(jwt.MapClaims)
	if !ok {
		log.Error("Failed to parse JWT claims")
		return
	}
	log.Info("", "JWT Claims", claims)

	signatureJson, err := json.Marshal(token.Signature)
	if err != nil {
		log.Error("Failed to parse JWT signature")
		return
	}
	log.Info("", "JWT Signature", string(signatureJson))

	method, err := json.Marshal(token.Method)
	if err != nil {
		log.Error("Failed to parse JWT method")
		return
	}
	log.Info("", "JWT Method", string(method))

	log.Info("", "JWT Valid", token.Valid)
}

func ConnectionsMiddleware(connections *auth.SafeConnectionMap) func(next http.Handler) http.Handler {
	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			ctx := auth.WithConnectionMap(r.Context(), connections)
			next.ServeHTTP(w, r.WithContext(ctx))
		})
	}
}

func JwtValidationMiddleware(cookieName string) func(next http.Handler) http.Handler {
	return func(next http.Handler) http.Handler {
		return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			token := r.Header.Get(cookieName)

			if token == "" {
				http.Error(w, "Unauthorized", http.StatusUnauthorized)
				return
			}

			next.ServeHTTP(w, r)
		})
	}
}
