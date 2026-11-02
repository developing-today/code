package main

import (
	"context"
	"errors"
	"os"
	"testing"
	"time"

	"github.com/charmbracelet/ssh"
)

func TestRunServerTreatsShutdownAsCleanStop(t *testing.T) {
	signals := make(chan os.Signal, 1)
	signals <- os.Interrupt

	serveRelease := make(chan struct{})
	shutdownCalled := false
	err := runServer(
		func() error {
			<-serveRelease
			return ssh.ErrServerClosed
		},
		func(context.Context) error {
			shutdownCalled = true
			close(serveRelease)
			return nil
		},
		signals,
		time.Second,
	)

	if err != nil {
		t.Fatalf("runServer() error = %v, want nil", err)
	}
	if !shutdownCalled {
		t.Fatal("runServer() did not shut down after receiving a signal")
	}
}

func TestRunServerReturnsServeFailure(t *testing.T) {
	serveErr := errors.New("listen tcp 127.0.0.1:23234: bind: address already in use")
	shutdownCalled := false
	err := runServer(
		func() error { return serveErr },
		func(context.Context) error {
			shutdownCalled = true
			return nil
		},
		make(chan os.Signal),
		time.Second,
	)

	if !errors.Is(err, serveErr) {
		t.Fatalf("runServer() error = %v, want %v", err, serveErr)
	}
	if shutdownCalled {
		t.Fatal("runServer() called shutdown after a serve failure")
	}
}
