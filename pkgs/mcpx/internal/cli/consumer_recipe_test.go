package cli

import (
	"context"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/daemon"
)

func TestRecipesRemoveCommandCallsDeleteEndpoint(t *testing.T) {
	requests := make(chan [2]string, 1)
	ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		requests <- [2]string{r.Method, r.URL.Path}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"name":"demo"}`))
	}))
	defer ts.Close()

	app := &App{
		client:    NewClientAt(daemon.Paths{}, "", ts.URL),
		connected: true,
	}
	if err := app.CmdRecipes(context.Background(), []string{"remove", "demo"}); err != nil {
		t.Fatalf("recipes remove: %v", err)
	}
	if got, want := <-requests, [2]string{http.MethodDelete, "/v1/recipes/demo"}; got != want {
		t.Fatalf("request = %v, want %v", got, want)
	}
}
