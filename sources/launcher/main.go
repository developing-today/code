package main

import (
	"fmt"
	"path/filepath"

	"github.com/charmbracelet/bubbles/spinner"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/keygen"
	"github.com/charmbracelet/log"
	"github.com/charmbracelet/ssh"
	"github.com/charmbracelet/wish"
	"github.com/charmbracelet/wish/activeterm"
	bm "github.com/charmbracelet/wish/bubbletea"
	lm "github.com/charmbracelet/wish/logging"
	"github.com/charmbracelet/wishlist"
)

func main() {
	k, err := keygen.New(filepath.Join(".wishlist", "server"), nil, keygen.WithKeyType(keygen.Ed25519))
	if err != nil {
		log.Fatal(err)
	}
	if !k.KeyPairExists() {
		if err := k.WriteKeys(); err != nil {
			log.Fatal(err)
		}
	}

	// wishlist config
	cfg := &wishlist.Config{
		Factory: func(e wishlist.Endpoint) (*ssh.Server, error) {
			return wish.NewServer(
				wish.WithAddress(e.Address),
				wish.WithHostKeyPEM(k.RawProtectedPrivateKey()),
				wish.WithPublicKeyAuth(func(ctx ssh.Context, key ssh.PublicKey) bool {
					return true
				}),
				wish.WithMiddleware(
					append(
						e.Middlewares, // this is the important bit: the middlewares from the endpoint
						lm.Middleware(),
						activeterm.Middleware(),
					)...,
				),
			)
		},
		Endpoints: []*wishlist.Endpoint{
			{
				Name: "bubbletea",
				Middlewares: []wish.Middleware{
					bm.Middleware(func(ssh.Session) (tea.Model, []tea.ProgramOption) {
						return initialModel(), nil
					}),
				},
			},
			{
				Name: "simple",
				Middlewares: []wish.Middleware{
					func(h ssh.Handler) ssh.Handler {
						return func(s ssh.Session) {
							_, _ = s.Write([]byte("hello, world\n\r"))
							h(s)
						}
					},
				},
			},
			{
				Name:    "app2",
				Address: "app.addr:2222",
			},
			{
				Name:    "server1",
				Address: "server1:22",
			},
			{
				Name:    "server2",
				Address: "server1:22",
				User:    "override_user",
			},
			{
				Name: "entries without middlewares and addresses are ignored",
			},
			{
				Address: "entries without names are ignored",
			},
		},
	}

	// start all the servers
	if err := wishlist.Serve(cfg); err != nil {
		log.Fatal(err)
	}
}

type model struct {
	spinner spinner.Model
}

func initialModel() model {
	s := spinner.NewModel()
	s.Spinner = spinner.Dot
	return model{spinner: s}
}

func (m model) Init() tea.Cmd {
	return spinner.Tick
}

func (m model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyMsg:
		log.Print("keypress:", msg)
		switch msg.String() {
		case "q", "esc", "ctrl+c":
			return m, tea.Quit
		}
	case tea.WindowSizeMsg:
		log.Print("window size:", msg)
	}
	var cmd tea.Cmd
	m.spinner, cmd = m.spinner.Update(msg)
	return m, cmd
}

func (m model) View() string {
	str := fmt.Sprintf("\n\n   %s Loading forever...press q to quit\n\n", m.spinner.View())
	return str
}

// package main

// import (
// 	"database/sql"
// 	"embed"
// 	"os"
// 	"strings"

// 	"github.com/charmbracelet/log"
// 	"github.com/pressly/goose/v3"

// 	_ "github.com/lib/pq"
// )

// //go:embed resources/general/migrations
// var embedMigrations embed.FS
// var resourceDir = "resources"
// var migrationsDir = "migrations"

// func main() {
// 	log.Info("Hello")
// 	baseConnString := os.Getenv("DATABASE_URL")
// 	if baseConnString == "" {
// 		log.Warn("DATABASE_URL is not set, using default")
// 		baseConnString = "postgresql://root@localhost:26258/defaultdb?sslcert=C:%5cUsers%5cdrewr%5ccerts-desktop-tower%5Cclient.root.crt&sslkey=C:%5cUsers%5cdrewr%5ccerts-desktop-tower%5Cclient.root.key&sslmode=verify-full&sslrootcert=C:%5cUsers%5cdrewr%5ccerts-desktop-tower%5Cca.crt"
// 	}

// 	db, err := sql.Open("postgres", baseConnString+"&application_name=$ data_migrations")
// 	if err != nil {
// 		log.Fatal(err)
// 	}
// 	defer db.Close()

// 	goose.SetBaseFS(embedMigrations)

// 	if err := goose.SetDialect("postgres"); err != nil {
// 		panic(err)
// 	}

// 	databaseDirs, err := embedMigrations.ReadDir(resourceDir)
// 	if err != nil {
// 		panic(err)
// 	}

// 	for _, databaseDir := range databaseDirs {
// 		databasePath := strings.Join([]string{resourceDir, databaseDir.Name()}, "/")
// 		log.Info("", "database", databasePath)
// 		if !databaseDir.IsDir() {
// 			log.Info("not a directory")
// 			continue
// 		}

// 		migrationPath := strings.Join([]string{databasePath, migrationsDir}, "/")

// 		migrationDirs, err := embedMigrations.ReadDir(migrationPath)
// 		if err != nil {
// 			panic(err)
// 		}

// 		for _, migrationDir := range migrationDirs {
// 			if !migrationDir.IsDir() {
// 				continue
// 			}
// 			migrationYearPath := strings.Join([]string{migrationPath, migrationDir.Name()}, "/")
// 			log.Info("", "migration-dir", migrationYearPath)

// 			migrationYearDirFiles, err := embedMigrations.ReadDir(migrationYearPath)
// 			if err != nil {
// 				panic(err)
// 			}
// 			for _, migrationYearDirFile := range migrationYearDirFiles {
// 				log.Info("", "migration-dir-file", migrationYearDirFile.Name())
// 			}

// 			log.Info("running migrations", "migration-dir", migrationYearPath)

// 			if err := goose.Up(db, migrationYearPath); err != nil {
// 				panic(err)
// 			}
// 		}
// 	}
// 	log.Info("Goodbye")
// }
