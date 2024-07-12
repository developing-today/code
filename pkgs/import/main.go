package main

import (
	"database/sql"
	"embed"
	"os"
	"strings"

	"github.com/charmbracelet/log"
	"github.com/pressly/goose/v3"

	_ "github.com/lib/pq"
)

//go:embed resources/general/migrations
var embedMigrations embed.FS
var resourceDir = "resources"
var migrationsDir = "migrations"

func main() {
	log.Info("Hello")
	baseConnString := os.Getenv("DATABASE_URL")
	if baseConnString == "" {
		log.Warn("DATABASE_URL is not set, using default")
		baseConnString = "postgresql://root@localhost:26258/defaultdb?sslcert=C:%5cUsers%5cdrewr%5ccerts-desktop-tower%5Cclient.root.crt&sslkey=C:%5cUsers%5cdrewr%5ccerts-desktop-tower%5Cclient.root.key&sslmode=verify-full&sslrootcert=C:%5cUsers%5cdrewr%5ccerts-desktop-tower%5Cca.crt"
	}

	db, err := sql.Open("postgres", baseConnString+"&application_name=$ data_migrations")
	if err != nil {
		log.Fatal(err)
	}
	defer db.Close()

	goose.SetBaseFS(embedMigrations)

	if err := goose.SetDialect("postgres"); err != nil {
		panic(err)
	}

	databaseDirs, err := embedMigrations.ReadDir(resourceDir)
	if err != nil {
		panic(err)
	}

	for _, databaseDir := range databaseDirs {
		databasePath := strings.Join([]string{resourceDir, databaseDir.Name()}, "/")
		log.Info("", "database", databasePath)
		if !databaseDir.IsDir() {
			log.Info("not a directory")
			continue
		}

		migrationPath := strings.Join([]string{databasePath, migrationsDir}, "/")

		migrationDirs, err := embedMigrations.ReadDir(migrationPath)
		if err != nil {
			panic(err)
		}

		for _, migrationDir := range migrationDirs {
			if !migrationDir.IsDir() {
				continue
			}
			migrationYearPath := strings.Join([]string{migrationPath, migrationDir.Name()}, "/")
			log.Info("", "migration-dir", migrationYearPath)

			migrationYearDirFiles, err := embedMigrations.ReadDir(migrationYearPath)
			if err != nil {
				panic(err)
			}
			for _, migrationYearDirFile := range migrationYearDirFiles {
				log.Info("", "migration-dir-file", migrationYearDirFile.Name())
			}

			log.Info("running migrations", "migration-dir", migrationYearPath)

			if err := goose.Up(db, migrationYearPath); err != nil {
				panic(err)
			}
		}
	}
	log.Info("Goodbye")
}
