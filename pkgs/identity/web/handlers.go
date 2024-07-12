package web

import (
	"fmt"
	"net/http"
	"strings"

	"github.com/angelofallars/htmx-go"
	"github.com/charmbracelet/log"

	"github.com/developing-today/code/src/identity/auth"
	"github.com/developing-today/code/src/identity/web/templates"
	"github.com/developing-today/code/src/identity/web/templates/pages"
)

func indexViewHandler(w http.ResponseWriter, r *http.Request) {
	// if r.URL.Path != "/" {
	// 	http.NotFound(w, r)
	// 	log.Error("render page", "method", r.Method, "status", http.StatusNotFound, "path", r.URL.Path)
	// 	return
	// }

	metaTags := pages.MetaTags(
		"identity server connections",
		"identity server connections",
	)
	bodyContent := pages.BodyContent(
		"identity server connections",
		"identity server connections",
	)
	indexTemplate := templates.Layout(
		"identity server connections",
		metaTags,
		bodyContent,
	)

	if err := htmx.NewResponse().RenderTempl(r.Context(), w, indexTemplate); err != nil {
		w.WriteHeader(http.StatusInternalServerError)
		log.Error("render template", "method", r.Method, "status", http.StatusInternalServerError, "path", r.URL.Path)
		return
	}

	log.Info("render page", "method", r.Method, "status", http.StatusOK, "path", r.URL.Path)
}

func showIDAPIHandler(w http.ResponseWriter, r *http.Request) {
	connections, ok := auth.GetConnectionMap(r.Context())
	if !ok {
		http.Error(w, "Could not get connections from context", http.StatusInternalServerError)
		return
	}

	var htmlContent strings.Builder
	htmlContent.WriteString("<table class=\"table-zebra\"><tr><th>ConnectionID</th><th>Status</th><th>CharmID</th><th>HTML</th></tr>")

	for _, connection := range connections.All() {
		htmlContent.WriteString(renderConnectionRow(&connection))
	}

	htmlContent.WriteString("</table>")

	w.Write([]byte(htmlContent.String()))
	htmx.NewResponse().Write(w)
	log.Info("request API", "method", r.Method, "status", http.StatusOK, "path", r.URL.Path)
}
func showIDAPIJsonHandler(w http.ResponseWriter, r *http.Request) {
	connections, ok := auth.GetConnectionMap(r.Context())
	if !ok {
		http.Error(w, "Could not get connections from context", http.StatusInternalServerError)
		return
	}

	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(http.StatusOK)

	jsonConnections, err := connections.JSON()

	if err != nil {
		http.Error(w, "Could not get JSON for connections", http.StatusInternalServerError)
		return
	}

	w.Write([]byte(jsonConnections))
	log.Info("request API", "method", r.Method, "status", http.StatusOK, "path", r.URL.Path)
}

func renderConnectionRow(c *auth.Connection) string {
	html, err := c.HTML()
	if err != nil {
		log.Error("Could not get HTML for connection", "connection", c, "error", err)
		return ""
	}
	return fmt.Sprintf("<tr><td>%v</td><td>%v</td><td>%v</td><td>%v</td></tr>", safeString(c.ConnectionID), safeString(c.Status), safeString(c.CharmID), html)
}

func safeString(s *string) string {
	if s != nil {
		return *s
	}
	return ""
}
