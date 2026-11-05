package main

import (
	"context"
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
	host = "localhost"
	port = 23234
)

// sender is the subset of tea.Program used for broadcast. Keeping the
// registry behind this interface lets tests exercise pruning and
// synchronization without running real terminal programs.
type sender interface {
	Send(tea.Msg)
}

// app contains a wish server and the registry of running programs.
// The registry is guarded by mu: sessions register on connect and are
// pruned when their program ends, so dead sessions neither accumulate
// nor receive fan-out traffic.
type app struct {
	*ssh.Server
	mu    sync.RWMutex
	progs []sender
}

// send dispatches a message to all running programs.
func (a *app) send(msg tea.Msg) {
	a.mu.RLock()
	progs := make([]sender, len(a.progs))
	copy(progs, a.progs)
	a.mu.RUnlock()
	for _, p := range progs {
		go p.Send(msg)
	}
}

// remove drops a finished program from the registry. Removing an
// unknown program is a no-op.
func (a *app) remove(p sender) {
	a.mu.Lock()
	defer a.mu.Unlock()
	for i, q := range a.progs {
		if q == p {
			a.progs = append(a.progs[:i], a.progs[i+1:]...)
			return
		}
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

func (a *app) Start() {
	var err error
	done := make(chan os.Signal, 1)
	signal.Notify(done, os.Interrupt, syscall.SIGINT, syscall.SIGTERM)
	log.Printf("Starting SSH server on %s:%d", host, port)
	go func() {
		if err = a.ListenAndServe(); err != nil {
			log.Fatalln(err)
		}
	}()

	<-done
	log.Println("Stopping SSH server")
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer func() { cancel() }()
	if err := a.Shutdown(ctx); err != nil {
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
	a.mu.Lock()
	a.progs = append(a.progs, p)
	a.mu.Unlock()
	// Deregister when the session ends, not via p.Wait(): wish calls
	// p.Run() — which initializes the channel Wait receives from — only
	// after this handler returns, so an early Wait can block on a nil
	// channel forever (and race with Run's initialization). The session
	// always outlives the program (wish runs p.Run() synchronously in
	// the session handler), so session-end pruning covers every session.
	go a.watchSession(s.Context(), p)

	return p
}

// watchSession removes p from the registry once ctx ends. It never touches
// the program itself, so it is safe to start before the program runs.
func (a *app) watchSession(ctx context.Context, p sender) {
	<-ctx.Done()
	a.remove(p)
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
