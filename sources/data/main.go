package main

import (
	"database/sql"
	"embed"
	"strings"

	"github.com/charmbracelet/log"
	"github.com/pressly/goose/v3"
)

//go:embed resources/general/migrations
var embedMigrations embed.FS
var resourceDir = "resources"
var migrationsDir = "migrations"

func main() {
	log.Info("Hello")
	var db *sql.DB
	// setup database

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
				log.Info("not a directory")
				continue
			}
			migrationYearPath := strings.Join([]string{migrationPath, migrationDir.Name()}, "/")
			log.Info("", "migration", migrationYearPath)

			migrationYearDirFiles, err := embedMigrations.ReadDir(migrationYearPath)
			if err != nil {
				panic(err)
			}
			for _, migrationYearDirFile := range migrationYearDirFiles {
				log.Info("", "migration-file", migrationYearDirFile.Name())
			}

			if err := goose.Up(db, migrationYearPath); err != nil {
				panic(err)
			}
		}
	}
}
