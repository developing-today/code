package main

import (
	"context"
	"errors"
	"fmt"
	"log"
	"os"
	"os/signal"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/charmbracelet/bubbles/textarea"
	"github.com/charmbracelet/bubbles/viewport"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/charmbracelet/ssh"
	"github.com/charmbracelet/wish"
	bm "github.com/charmbracelet/wish/bubbletea"
	lm "github.com/charmbracelet/wish/logging"
	"github.com/muesli/termenv"
)

const (
	host                = "localhost"
	port                = 23234
	recipientQueueDepth = 16
)

// sender is the subset of tea.Program used for broadcast. Keeping the
// registry behind this interface lets tests exercise pruning and
// synchronization without running real terminal programs.
type sender interface {
	Send(tea.Msg)
}

// recipientQueue serializes delivery to one program. It keeps at most
// recipientQueueDepth messages queued behind the single in-flight Send.
type recipientQueue struct {
	program sender
	queue   chan tea.Msg
	stopped chan struct{}
	mu      sync.Mutex
	closed  bool
}

func newRecipientQueue(p sender) *recipientQueue {
	r := &recipientQueue{
		program: p,
		queue:   make(chan tea.Msg, recipientQueueDepth),
		stopped: make(chan struct{}),
	}
	go r.run()
	return r
}

func (r *recipientQueue) enqueue(msg tea.Msg) bool {
	r.mu.Lock()
	defer r.mu.Unlock()
	if r.closed {
		return false
	}
	select {
	case r.queue <- msg:
		return true
	default:
		// A stalled recipient is pruned rather than allowing its backlog or
		// blocked Send goroutines to grow with room traffic.
		r.closed = true
		close(r.stopped)
		return false
	}
}

func (r *recipientQueue) stop() {
	r.mu.Lock()
	defer r.mu.Unlock()
	if !r.closed {
		r.closed = true
		close(r.stopped)
	}
}

func (r *recipientQueue) run() {
	for {
		select {
		case <-r.stopped:
			return
		default:
		}

		select {
		case <-r.stopped:
			return
		case msg := <-r.queue:
			select {
			case <-r.stopped:
				return
			default:
			}
			r.program.Send(msg)
		}
	}
}

// app contains a wish server and the registry of running programs.
// The registry is guarded by mu: sessions register on connect and are
// pruned when their program ends, so dead sessions neither accumulate
// nor receive fan-out traffic.
type app struct {
	*ssh.Server
	mu         sync.RWMutex
	sendMu     sync.Mutex
	progs      []sender
	dispatches map[sender]*recipientQueue
}

// register adds a running program and its ordered delivery worker.
func (a *app) register(p sender) {
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.dispatches == nil {
		a.dispatches = make(map[sender]*recipientQueue)
	}
	if _, exists := a.dispatches[p]; exists {
		return
	}
	a.progs = append(a.progs, p)
	a.dispatches[p] = newRecipientQueue(p)
}

// send enqueues a message for each running program in a single broadcast
// order. A recipient can have one in-flight Send and at most
// recipientQueueDepth queued messages; a full queue prunes that recipient.
func (a *app) send(msg tea.Msg) {
	a.sendMu.Lock()
	defer a.sendMu.Unlock()

	a.mu.RLock()
	progs := append([]sender(nil), a.progs...)
	dispatches := make([]*recipientQueue, len(progs))
	for i, p := range progs {
		dispatches[i] = a.dispatches[p]
	}
	a.mu.RUnlock()
	for i, dispatch := range dispatches {
		if dispatch == nil || !dispatch.enqueue(msg) {
			a.remove(progs[i])
		}
	}
}

// remove drops a finished program from the registry. Removing an
// unknown program is a no-op.
func (a *app) remove(p sender) {
	a.mu.Lock()
	var dispatch *recipientQueue
	for i, q := range a.progs {
		if q == p {
			a.progs = append(a.progs[:i], a.progs[i+1:]...)
			dispatch = a.dispatches[p]
			delete(a.dispatches, p)
			break
		}
	}
	a.mu.Unlock()
	if dispatch != nil {
		dispatch.stop()
	}
}

func newApp() *app {
	a := new(app)

	s, err := wish.NewServer(
		wish.WithAddress(fmt.Sprintf("%s:%d", host, port)),
		wish.WithHostKeyPath(".ssh/term_info_ed25519"),
		wish.WithMiddleware(
			bm.MiddlewareWithProgramHandler(a.ProgramHandler, termenv.ANSI256),
			lm.Middleware(),
		),
	)

	if err != nil {
		log.Fatalln(err)
	}

	a.Server = s
	return a
}

func runServer(serve func() error, shutdown func(context.Context) error, done <-chan os.Signal, timeout time.Duration) error {
	serveErr := make(chan error, 1)
	go func() {
		serveErr <- serve()
	}()

	select {
	case err := <-serveErr:
		if err != nil && !errors.Is(err, ssh.ErrServerClosed) {
			return err
		}
		return nil
	case <-done:
		log.Println("Stopping SSH server")
		ctx, cancel := context.WithTimeout(context.Background(), timeout)
		defer cancel()
		if err := shutdown(ctx); err != nil {
			return err
		}

		err := <-serveErr
		if err != nil && !errors.Is(err, ssh.ErrServerClosed) {
			return err
		}
		return nil
	}
}

func (a *app) Start() {
	done := make(chan os.Signal, 1)
	signal.Notify(done, os.Interrupt, syscall.SIGINT, syscall.SIGTERM)
	defer signal.Stop(done)
	log.Printf("Starting SSH server on %s:%d", host, port)
	if err := runServer(a.ListenAndServe, a.Shutdown, done, 30*time.Second); err != nil {
		log.Fatalln(err)
	}
}

func (a *app) ProgramHandler(s ssh.Session) *tea.Program {
	if _, _, active := s.Pty(); !active {
		wish.Fatalln(s, "terminal is not active")
	}

	model := initialModel()
	model.app = a
	model.id = s.User()

	p := tea.NewProgram(model, tea.WithOutput(s), tea.WithInput(s))
	a.register(p)
	go func() {
		p.Wait()
		a.remove(p)
	}()

	return p
}

func main() {
	app := newApp()
	app.Start()
}

type (
	errMsg  error
	chatMsg struct {
		id   string
		text string
	}
)

type model struct {
	*app
	viewport    viewport.Model
	messages    []string
	id          string
	textarea    textarea.Model
	senderStyle lipgloss.Style
	err         error
}

func initialModel() model {
	ta := textarea.New()
	ta.Placeholder = "Send a message..."
	ta.Focus()

	ta.Prompt = "┃ "
	ta.CharLimit = 280

	ta.SetWidth(30)
	ta.SetHeight(3)

	// Remove cursor line styling
	ta.FocusedStyle.CursorLine = lipgloss.NewStyle()

	ta.ShowLineNumbers = false

	vp := viewport.New(30, 5)
	vp.SetContent(`Welcome to the chat room!
Type a message and press Enter to send.`)

	ta.KeyMap.InsertNewline.SetEnabled(false)

	return model{
		textarea:    ta,
		messages:    []string{},
		viewport:    vp,
		senderStyle: lipgloss.NewStyle().Foreground(lipgloss.Color("5")),
		err:         nil,
	}
}

func (m model) Init() tea.Cmd {
	return textarea.Blink
}

func (m model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	var (
		tiCmd tea.Cmd
		vpCmd tea.Cmd
	)

	m.textarea, tiCmd = m.textarea.Update(msg)
	m.viewport, vpCmd = m.viewport.Update(msg)

	switch msg := msg.(type) {
	case tea.KeyMsg:
		switch msg.Type {
		case tea.KeyCtrlC, tea.KeyEsc:
			return m, tea.Quit
		case tea.KeyEnter:
			// An empty or whitespace-only input sends nothing (#3273):
			// broadcasting it would append a bare "name: " line to every
			// session's history. The textarea is left exactly as it is.
			// Real text, including surrounding spaces, is sent verbatim.
			if strings.TrimSpace(m.textarea.Value()) != "" {
				m.app.send(chatMsg{
					id:   m.id,
					text: m.textarea.Value(),
				})
				m.textarea.Reset()
			}
		}

	case chatMsg:
		m.messages = append(m.messages, m.senderStyle.Render(msg.id)+": "+msg.text)
		m.viewport.SetContent(strings.Join(m.messages, "\n"))
		m.viewport.GotoBottom()

	// We handle errors just like any other message
	case errMsg:
		m.err = msg
		return m, nil
	}

	return m, tea.Batch(tiCmd, vpCmd)
}

func (m model) View() string {
	return fmt.Sprintf(
		"%s\n\n%s",
		m.viewport.View(),
		m.textarea.View(),
	) + "\n\n"
}
