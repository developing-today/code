// Command placement-hard-stop maintains the steward-owned hard-stops.json
// registry consumed by placement-record. Its entire command line is parsed
// and validated before any registry file is read or written (#3352).
//
// Usage:
//
//	placement-hard-stop --action set|clear --file <hard-stops.json> --pool <serviceId> --note <reason> [--issued-at <RFC3339>]
package main

import (
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"strings"
	"time"

	"github.com/developing-today/code/pkgs/fleet/placement"
)

func main() {
	if err := run(os.Args[1:], time.Now()); err != nil {
		fmt.Fprintln(os.Stderr, "placement-hard-stop:", err)
		var usage *usageError
		if errors.As(err, &usage) {
			os.Exit(2)
		}
		os.Exit(1)
	}
}

type usageError struct {
	err error
}

func (e *usageError) Error() string { return e.err.Error() }
func (e *usageError) Unwrap() error { return e.err }

func invalidUsage(format string, args ...any) error {
	return &usageError{err: fmt.Errorf(format, args...)}
}

func run(args []string, now time.Time) error {
	flags := flag.NewFlagSet("placement-hard-stop", flag.ContinueOnError)
	flags.SetOutput(io.Discard)
	action := flags.String("action", "", "set or clear the pool hard-stop")
	path := flags.String("file", "", "path to hard-stops.json")
	pool := flags.String("pool", "", "pool serviceId to update")
	note := flags.String("note", "", "auditable reason for setting or clearing the stop")
	issuedAt := flags.String("issued-at", "", "RFC3339 timestamp (defaults to current UTC time)")
	if err := flags.Parse(args); err != nil {
		if errors.Is(err, flag.ErrHelp) {
			return nil
		}
		return invalidUsage("%w\nusage: placement-hard-stop --action set|clear --file <hard-stops.json> --pool <serviceId> --note <reason> [--issued-at <RFC3339>]", err)
	}
	if flags.NArg() != 0 {
		return invalidUsage("unexpected argument %q; usage: placement-hard-stop --action set|clear --file <hard-stops.json> --pool <serviceId> --note <reason> [--issued-at <RFC3339>]", flags.Arg(0))
	}
	provided := make(map[string]bool)
	flags.Visit(func(f *flag.Flag) { provided[f.Name] = true })
	if *action != "set" && *action != "clear" {
		return invalidUsage("--action must be set or clear")
	}
	if !provided["file"] || strings.TrimSpace(*path) == "" {
		return invalidUsage("--file is required and must not be empty")
	}
	if !provided["pool"] || strings.TrimSpace(*pool) == "" {
		return invalidUsage("--pool is required and must not be empty")
	}
	if !provided["note"] || strings.TrimSpace(*note) == "" {
		return invalidUsage("--note is required and must not be empty")
	}
	at := now.UTC()
	if provided["issued-at"] {
		if strings.TrimSpace(*issuedAt) == "" {
			return invalidUsage("--issued-at was supplied with an empty value")
		}
		parsed, err := time.Parse(time.RFC3339, *issuedAt)
		if err != nil {
			return invalidUsage("--issued-at must be RFC3339: %w", err)
		}
		at = parsed
	}
	stopped := *action == "set"
	return placement.WriteHardStop(*path, *pool, *note, stopped, at)
}
