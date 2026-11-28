package cli

import (
	"context"
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"github.com/developing-today/code/pkgs/mcpx/internal/openapi"
)

// The Dropbox, Google Drive and Outlook declarations shipped in
// examples/ are the #1936 remainder-2 deliverables: API declarations
// over each service's HTTP API, in the same shipped-example form as
// the tailscale adapter declaration. Each carries exactly one
// credential shape — an Authorization header whose value is a
// ${VAR} placeholder the loader expands from the environment — so a
// declaration can be checked in and a token cannot. Each shipped
// OpenAPI document is a curated read-only subset (the vendor
// documents are a discovery document, a Stone spec and a
// tens-of-megabytes Graph document respectively), and the tool set
// it builds to is pinned here so the shipped files cannot rot into
// a shape the loader would reject or a surface wider than the
// request asked for.
func TestShippedAPIDeclarations(t *testing.T) {
	examples := filepath.Join("..", "..", "examples")
	cases := []struct {
		service  string
		name     string
		tokenVar string
		methods  []string
		tools    []string
		verb     string
	}{
		{
			// Dropbox API v2 is RPC over POST: the declaration sets
			// methods deliberately, and every operation in the
			// document is still a read.
			service: "dropbox", name: "dropbox", tokenVar: "DROPBOX_ACCESS_TOKEN",
			methods: []string{"POST"}, verb: "POST",
			tools: []string{
				"dropbox_getcurrentaccount", "dropbox_getmetadata",
				"dropbox_listfolder", "dropbox_listfoldercontinue",
				"dropbox_searchfiles",
			},
		},
		{
			service: "google-drive", name: "googledrive", tokenVar: "GOOGLE_DRIVE_ACCESS_TOKEN",
			verb: "GET",
			tools: []string{
				"googledrive_getabout", "googledrive_getfile",
				"googledrive_listdrives", "googledrive_listfiles",
			},
		},
		{
			service: "outlook", name: "outlook", tokenVar: "OUTLOOK_ACCESS_TOKEN",
			verb: "GET",
			tools: []string{
				"outlook_getmessage", "outlook_getprofile",
				"outlook_listevents", "outlook_listmailfolders",
				"outlook_listmessages",
			},
		},
	}
	for _, tc := range cases {
		t.Run(tc.service, func(t *testing.T) {
			declPath := filepath.Join(examples, tc.service+".json")
			specs, err := loadAPIFiles([]string{declPath})
			if err != nil {
				t.Fatalf("shipped %s declaration does not load: %v", tc.service, err)
			}
			if len(specs) != 1 {
				t.Fatalf("got %d APIs, want exactly 1", len(specs))
			}
			spec := specs[0]
			if spec.Name != tc.name {
				t.Fatalf("API is %q, want %q", spec.Name, tc.name)
			}
			if spec.Spec != "./"+tc.service+".openapi.json" {
				t.Fatalf("spec is %q, want the shipped ./%s.openapi.json", spec.Spec, tc.service)
			}
			if spec.BaseURL != "" {
				t.Fatalf("declaration must take its base URL from the document, got %q", spec.BaseURL)
			}
			if !slices.Equal(spec.Methods, tc.methods) {
				t.Fatalf("methods are %v, want %v", spec.Methods, tc.methods)
			}
			if len(spec.Headers) != 1 {
				t.Fatalf("declaration must carry exactly one header, got %+v", spec.Headers)
			}
			auth, ok := spec.Headers["Authorization"]
			if !ok || auth != "Bearer ${"+tc.tokenVar+"}" {
				t.Fatalf("Authorization header is %q, want the Bearer ${%s} placeholder — "+
					"a declaration is checked in, a token is not", auth, tc.tokenVar)
			}

			doc, err := openapi.Load(context.Background(),
				filepath.Join(examples, tc.service+".openapi.json"), 0)
			if err != nil {
				t.Fatalf("shipped %s document does not load: %v", tc.service, err)
			}
			api, err := openapi.Build(doc, openapi.Options{
				Name: spec.Name, Include: spec.Include, Exclude: spec.Exclude,
				Methods: spec.Methods, IncludeDeprecated: spec.IncludeDeprecated,
				Headers: spec.Headers,
			})
			if err != nil {
				t.Fatalf("shipped %s document does not build: %v", tc.service, err)
			}
			var names []string
			for _, tool := range api.Tools {
				names = append(names, tool.Name)
				if tool.Method != tc.verb {
					t.Fatalf("tool %s is a %s, want %s only", tool.Name, tool.Method, tc.verb)
				}
				if strings.TrimSpace(tool.Description) == "" {
					t.Fatalf("tool %s has no description", tool.Name)
				}
			}
			slices.Sort(names)
			if !slices.Equal(names, tc.tools) {
				t.Fatalf("tools are %v, want exactly %v", names, tc.tools)
			}
		})
	}
}
