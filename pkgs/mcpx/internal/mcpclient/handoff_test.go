package mcpclient

import (
	"context"
	"strings"
	"testing"
	"time"
)

const echoScript = `while IFS= read -r l; do printf '%s\n' "$l"; done`

func TestDetachedChildKeepsItsPipesAndAdoptionResumesThem(t *testing.T) {
	first, err := NewStdio(StdioOptions{Command: "sh", Args: []string{"-c", echoScript}, InheritEnv: true})
	if err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	if err := first.Send(ctx, []byte(`{"n":1}`)); err != nil {
		t.Fatal(err)
	}
	if got, err := first.Recv(); err != nil || string(got) != `{"n":1}` {
		t.Fatalf("before handoff: %q %v", got, err)
	}

	h, err := first.Detach()
	if err != nil {
		t.Fatal(err)
	}
	if _, err := first.Detach(); err == nil {
		t.Error("a second Detach of the same transport succeeded")
	}
	if err := first.Send(ctx, []byte(`{"n":2}`)); err == nil {
		t.Error("Send succeeded on a detached transport")
	}

	second, err := AdoptStdio(*h)
	if err != nil {
		t.Fatal(err)
	}
	if second.PID() != h.PID {
		t.Fatalf("adopted pid %d, want the child's %d", second.PID(), h.PID)
	}
	if err := second.Send(ctx, []byte(`{"n":3}`)); err != nil {
		t.Fatal(err)
	}
	if got, err := second.Recv(); err != nil || string(got) != `{"n":3}` {
		t.Fatalf("after adoption: %q %v", got, err)
	}
	if !childAlive(h.PID, h.StartTime) {
		t.Fatal("child not alive after adoption")
	}
	_ = second.Close()
	<-second.exited
}

func TestHandoffDoesNotSplitAFrameAcrossTheTwoProcesses(t *testing.T) {
	script := `printf '{"par'; sleep 1; printf 'tial":1}\n'; ` + echoScript
	first, err := NewStdio(StdioOptions{Command: "sh", Args: []string{"-c", script}, InheritEnv: true})
	if err != nil {
		t.Fatal(err)
	}
	got := make(chan []byte, 1)
	go func() {
		line, _ := first.Recv()
		got <- line
	}()
	time.Sleep(200 * time.Millisecond)
	h, err := first.Detach()
	if err != nil {
		t.Fatal(err)
	}
	if line := <-got; line != nil {
		t.Fatalf("a read interrupted mid-frame returned %q", line)
	}

	second, err := AdoptStdio(*h)
	if err != nil {
		t.Fatal(err)
	}
	line, err := second.Recv()
	if err != nil || string(line) != `{"partial":1}` {
		t.Fatalf("frame after handoff: %q %v", line, err)
	}
	_ = second.Close()
	<-second.exited
}

func TestAdoptRefusesAChildThatEnded(t *testing.T) {
	tr, err := NewStdio(StdioOptions{Command: "sh", Args: []string{"-c", "exit 0"}, InheritEnv: true})
	if err != nil {
		t.Fatal(err)
	}
	<-tr.exited
	_, err = AdoptStdio(StdioHandoff{PID: tr.PID(), StartTime: 0, Label: "gone"})
	if err == nil || !strings.Contains(err.Error(), "not running") {
		t.Fatalf("adopting an ended child: %v", err)
	}
}
