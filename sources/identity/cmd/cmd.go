package cmd

import (
	"context"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"net"
	"os"
	"os/signal"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/charmbracelet/bubbles/key"
	"github.com/charmbracelet/bubbles/spinner"
	"github.com/charmbracelet/bubbles/viewport"
	tea "github.com/charmbracelet/bubbletea"
	charmcmd "github.com/charmbracelet/charm/cmd"
	"github.com/charmbracelet/lipgloss"
	"github.com/charmbracelet/log"
	"github.com/charmbracelet/melt"
	"github.com/charmbracelet/promwish"
	"github.com/charmbracelet/ssh"
	"github.com/charmbracelet/wish"
	"github.com/charmbracelet/wish/bubbletea"
	"github.com/charmbracelet/wish/comment"
	elapsed "github.com/charmbracelet/wish/elapsed"
	"github.com/charmbracelet/wish/logging"
	"github.com/charmbracelet/wish/scp"
	"github.com/developing-today/code/src/identity/auth"
	"github.com/developing-today/code/src/identity/configuration"
	"github.com/developing-today/code/src/identity/observability"
	"github.com/developing-today/code/src/identity/stream"
	"github.com/developing-today/code/src/identity/web"
	"github.com/knadh/koanf"
	"github.com/muesli/reflow/wordwrap"
	"github.com/muesli/reflow/wrap"
	"github.com/prometheus/client_golang/prometheus"
	"github.com/prometheus/client_golang/prometheus/promauto"
	"github.com/prometheus/client_golang/prometheus/promhttp"
	"github.com/samber/do/v2"
	"github.com/spf13/cobra"
	gossh "golang.org/x/crypto/ssh"
)

/*
// todo
embed default kdl file,
default kdl file ->
hard code vars in build ->
config file -> env vars ->
remote config (

	s3 ->
	db ->
	nats/centrifuge ->
	etc) (
	dont do all this, just this is the direction eventually as things become available if)
*/

// todo: put these into configuration but also as flat defaults in configuration
var Separator = "."
var ConfigurationFilePath = "config.kdl"
var EmbeddedConfigurationFilePath = "embed/config.kdl"
var GeneratedKeyDirPath = ".ssh/generated"
var HostKeyPath = ".ssh/term_info_ed25519"
var ScpFileSystemDirPath = "scp"

func NewConfiguration() *configuration.IdentityServerConfiguration {
	return &configuration.IdentityServerConfiguration{
		Configuration: koanf.New(Separator),
		ConfigurationLocations: &configuration.ConfigurationLocations{
			ConfigurationFilePaths: []string{
				ConfigurationFilePath,
				// identity.kdl identity.config.kdl config.identity.kdl identity.config
				// run these against ? binary dir ? pwd of execution ? appdata ? .config ? .local ???
				// then check for further locations/env-prefixes/etc from first pass, rerun on top with second pass
				// (maybe config.kdl next to binary sets a new set of configurationPaths, finish out loading from defaults, then load from new paths)
				// this pattern continues, after hard-code default env/file search, then custom file/env search, then eventually maybe nats/centrifuge/s3 or other remote or db config
			},
			EmbeddedConfigurationFilePaths: []string{
				EmbeddedConfigurationFilePath,
			},
		},
		EmbedFS: &configuration.EmbedFS,
	}
}

func WrappedCharmFromContext(ctx context.Context, config *configuration.IdentityServerConfiguration) *cobra.Command {
	if ctx == nil {
		ctx = context.Background()
	}
	if config == nil {
		panic("config is nil")
	}
	cmd := charmcmd.RootCmd

	go func() {
		<-ctx.Done()

		p, err := os.FindProcess(os.Getpid()) // TODO: fix this remove it or something, this doesn't work on windows
		if err != nil {
			log.Error("could not find process", "error", err)
			return
		}
		if err := p.Signal(syscall.SIGINT); err != nil {
			log.Error("could not send interrupt signal", "error", err)
		}
	}()

	return cmd
}

func CharmCmd(ctx context.Context, config *configuration.IdentityServerConfiguration) *cobra.Command {
	if ctx == nil {
		ctx = context.Background()
	}
	if config == nil {
		panic("config is nil")
	}
	return WrappedCharmFromContext(ctx, config)
}

func StartCharmCmd(ctx context.Context, config *configuration.IdentityServerConfiguration) *cobra.Command {
	if ctx == nil {
		ctx = context.Background()
	}
	if config == nil {
		panic("config is nil")
	}
	result := WrappedCharmFromContext(ctx, config)
	result.Use = "charm"
	result.Aliases = []string{"ch", "c"}
	return result
}

func StartAllAltCmd(command cobra.Command) *cobra.Command {
	result := command
	result.Use = "all"
	result.Aliases = []string{"al", "a"}
	return &result
}

func LoadDefaultConfiguration() *configuration.IdentityServerConfiguration {
	config := NewConfiguration()
	config.LoadConfiguration()
	log.Info("Loaded config", "config", config.Configuration.Sprint())
	return config
}

func DefaultRootCmd() *cobra.Command {
	return RootCmd(context.Background(), LoadDefaultConfiguration())
}

func DefaultRootCmdWithContext(ctx context.Context) *cobra.Command {
	if ctx == nil {
		ctx = context.Background()
	}
	return RootCmd(ctx, LoadDefaultConfiguration())
}

func RootCmd(ctx context.Context, config *configuration.IdentityServerConfiguration) *cobra.Command {
	if ctx == nil {
		ctx = context.Background()
	}
	if config == nil {
		panic("config is nil")
	}
	result := &cobra.Command{
		Use:   "identity",
		Short: "publish your identity",
		Long:  `publish your identity and allow others to connect to you.`,
	}
	result.AddCommand(charmcmd.RootCmd, StartAllCmd(ctx, config))
	return result
}

func StartAllCmd(ctx context.Context, config *configuration.IdentityServerConfiguration) *cobra.Command {
	if ctx == nil {
		ctx = context.Background()
	}
	if config == nil {
		panic("config is nil")
	}
	result := &cobra.Command{
		Use:     "start",
		Short:   "Starts the identity and charm servers",
		Run:     StartAllTasks(ctx, config),
		Aliases: []string{"s", "run", "serve", "publish", "pub", "p", "i", "y", "u", "o", "p", "q", "w", "e", "r", "t", "a", "s", "d", "f", "g", "h", "j", "k", "l", "z", "x", "c", "v", "b"},
	}
	result.AddCommand(StartCharmCmd(ctx, config), StartIdentityCmd(ctx, config), StartStreamCmd(ctx, config))
	result.AddCommand(StartAllAltCmd(*result))
	return result
}

func StartIdentityCmd(ctx context.Context, config *configuration.IdentityServerConfiguration) *cobra.Command {
	if ctx == nil {
		ctx = context.Background()
	}
	if config == nil {
		panic("config is nil")
	}
	return &cobra.Command{
		Use:     "identity",
		Short:   "Starts only the identity server",
		Run:     StartIdentityFromContext(ctx, config),
		Aliases: []string{"id", "i"},
	}
}

func StartStreamCmd(ctx context.Context, config *configuration.IdentityServerConfiguration) *cobra.Command {
	if ctx == nil {
		ctx = context.Background()
	}
	if config == nil {
		panic("config is nil")
	}
	return &cobra.Command{
		Use:     "stream",
		Short:   "Starts only the stream server",
		Run:     StartStreamFromContext(ctx, config),
		Aliases: []string{"tr", "t"},
	}
}

func GetTasks(ctx context.Context, config *configuration.IdentityServerConfiguration) []func(context.Context, *sync.WaitGroup) func(*cobra.Command, []string) {
	if ctx == nil {
		ctx = context.Background()
	}
	if config == nil {
		panic("config is nil")
	}
	return []func(ictx context.Context, wg *sync.WaitGroup) func(*cobra.Command, []string){
		StartStream(config),
		StartCharm(config),
		StartIdentity(config),
	}
}

func StartAllTasks(ctx context.Context, config *configuration.IdentityServerConfiguration) func(*cobra.Command, []string) {
	if ctx == nil {
		ctx = context.Background()
	}
	if config == nil {
		panic("config is nil")
	}
	return func(cmd *cobra.Command, args []string) {
		if ctx == nil {
			ctx = context.Background()
		}
		StartTasks(ctx, GetTasks(ctx, config)...)(cmd, args)
	}
}

//	type Application struct {
//		MustInvoke []*Application
//		Name       *string
//		Service    interface{}
//		tasks      []func(context.Context, *sync.WaitGroup) func(*cobra.Command, []string)
//	}
//
//	type Application[T any] struct {
//		Service T
//		Compile func(*cobra.Command, []string)
//	}
type Application[T any] interface {
	Service() T
	Compile(*cobra.Command, []string) func(*cobra.Command, []string)
}
type Service[T any] interface {
	Provide(injector *do.Injector) error
	Invoke(injector *do.Injector) T
}

type JwtVerifierService struct {
}
type JwtVerifierApp struct {
	Application[JwtVerifierService]
}

func StartTasks(ctx context.Context, tasks ...func(context.Context, *sync.WaitGroup) func(*cobra.Command, []string)) func(*cobra.Command, []string) {
	if ctx == nil {
		ctx = context.Background()
	}
	return func(cmd *cobra.Command, args []string) {
		if ctx == nil {
			ctx = context.Background()
		}
		ctx, cancel := context.WithCancel(ctx)
		defer cancel()

		var wg sync.WaitGroup
		wg.Add(len(tasks))

		for _, task := range tasks {
			go func(task func(context.Context, *sync.WaitGroup) func(*cobra.Command, []string)) {
				defer wg.Done()
				defer cancel()
				task(ctx, &wg)(cmd, args)
			}(task)
		}

		c := make(chan os.Signal, 1)
		// signal.Notify(c, os.Interrupt) // , syscall.SIGTERM, syscall.SIGINT, syscall.SIGQUIT // , os.Kill // todo: this needs to let me force kill it, maybe take out os.kill or something?
		// defer signal.Stop(c)

		select {
		case <-c:
			log.Info("SIGINT received, cancelling tasks.")
			cancel()
		case <-ctx.Done():
			log.Info("Context cancelled, ensuring all tasks complete.")
		}

		wg.Wait()
		log.Info("All tasks have completed or been cancelled. Proceeding to cleanup and shutdown.")
		FinalShutdown(ctx, cmd, args, tasks...)
	}
}

func CleanupAndShutdown(cancel context.CancelFunc, done chan struct{}) {
	log.Info("Cleaning up and shutting down.")
	cancel()
	log.Info("Cancelled context. waiting for tasks to complete.")
	<-done
	log.Info("All tasks done. Shutting down.")
}

func FinalShutdown(ctx context.Context, cmd *cobra.Command, args []string, tasks ...func(context.Context, *sync.WaitGroup) func(*cobra.Command, []string)) {
	if ctx == nil {
		ctx = context.Background()
	}
	if cmd == nil {
		panic("cmd is nil")
	}
	log.Info("All tasks cleaned up. Shutting down.", "len(tasks)", len(tasks), "command", cmd.Name(), "args", args)
	log.Info("Bye!", "time", time.Now())
}

func RunTask(ctx context.Context, wg *sync.WaitGroup, taskFunc func(context.Context, *cobra.Command, []string)) func(*cobra.Command, []string) {
	if ctx == nil {
		ctx = context.Background()
	}
	return func(cmd *cobra.Command, args []string) {
		if ctx == nil {
			ctx = context.Background()
		}
		defer wg.Done()
		taskFunc(ctx, cmd, args)
	}
}

func StartStreamFromContext(ctx context.Context, config *configuration.IdentityServerConfiguration) func(*cobra.Command, []string) {
	if ctx == nil {
		ctx = context.Background()
	}
	if config == nil {
		panic("config is nil")
	}
	return func(cmd *cobra.Command, args []string) {
		StartStream(config)(ctx, nil)(cmd, args)
	}
}

func StartStream(config *configuration.IdentityServerConfiguration) func(context.Context, *sync.WaitGroup) func(*cobra.Command, []string) {
	if config == nil {
		panic("config is nil")
	}
	return func(ctx context.Context, wg *sync.WaitGroup) func(*cobra.Command, []string) {
		return func(cmd *cobra.Command, args []string) {
			log.Info("Starting stream server")
			if ctx == nil {
				ctx = context.Background()
			}
			stream.RunStreamServer(ctx, config, cmd, args)
		}
	}
}
func StartCharmFromContext(ctx context.Context, config *configuration.IdentityServerConfiguration) func(*cobra.Command, []string) {
	if ctx == nil {
		ctx = context.Background()
	}
	if config == nil {
		panic("config is nil")
	}
	return func(cmd *cobra.Command, args []string) {
		StartCharm(config)(ctx, nil)(cmd, args)
	}
}

func StartCharm(config *configuration.IdentityServerConfiguration) func(context.Context, *sync.WaitGroup) func(*cobra.Command, []string) {
	if config == nil {
		panic("config is nil")
	}
	return func(ctx context.Context, wg *sync.WaitGroup) func(*cobra.Command, []string) {
		return func(cmd *cobra.Command, args []string) {
			log.Info("Starting charm server")
			if err := charmcmd.ServeCmdRunEWithContext(ctx, cmd, args); err != nil {
				log.Error("Error running charm server command", "error", err)
			}
		}
	}
}

func StartIdentityFromContext(ctx context.Context, config *configuration.IdentityServerConfiguration) func(*cobra.Command, []string) {
	if ctx == nil {
		ctx = context.Background()
	}
	if config == nil {
		panic("config is nil")
	}
	return func(cmd *cobra.Command, args []string) {
		StartIdentity(config)(ctx, nil)(cmd, args)
	}
}

var keyTypeCounter = promauto.NewCounterVec(prometheus.CounterOpts{
	Name: "wish_auth_by_type_total",
	Help: "The total number of authentications by type",
}, []string{"type"})

func StartIdentity(config *configuration.IdentityServerConfiguration) func(context.Context, *sync.WaitGroup) func(*cobra.Command, []string) {
	if config == nil {
		panic("config is nil")
	}
	return func(goctx context.Context, wg *sync.WaitGroup) func(*cobra.Command, []string) {
		return func(cmd *cobra.Command, args []string) {
			log.Info("Starting identity server")
			if goctx == nil {
				goctx = context.Background()
			}
			connections := auth.NewSafeConnectionMap()
			web.GoRunWebServer(goctx, connections, config)
			handler := scp.NewFileSystemHandler(ScpFileSystemDirPath)
			registry := prometheus.NewRegistry()

			s, err := wish.NewServer(
				wish.WithMiddleware(
					scp.Middleware(handler, handler),
					bubbletea.Middleware(TeaHandler),
					comment.Middleware("Thanks, have a nice day!"),
					elapsed.Middleware(),
					promwish.MiddlewareRegistry(
						registry,
						prometheus.Labels{
							"app": "identity",
						},
						promwish.DefaultCommandFn,
					),
					logging.Middleware(),
					observability.Middleware(connections),
				),
				wish.WithPasswordAuth(func(ctx ssh.Context, password string) bool {
					log.Info("Accepting password", "password", password, "len", len(password))
					return Connect(ctx, nil, &password, nil, connections)
				}),
				wish.WithKeyboardInteractiveAuth(func(ctx ssh.Context, challenge gossh.KeyboardInteractiveChallenge) bool {
					log.Info("Accepting keyboard interactive")
					return Connect(ctx, nil, nil, challenge, connections)
				}),
				wish.WithPublicKeyAuth(func(ctx ssh.Context, key ssh.PublicKey) bool {
					log.Info("Accepting public key", "publicKeyType", key.Type(), "publicKeyString", base64.StdEncoding.EncodeToString(key.Marshal()))
					return Connect(ctx, key, nil, nil, connections)
				}),
				wish.WithBannerHandler(Banner(config)),
				wish.WithAddress(fmt.Sprintf("%s:%d", config.Configuration.String("identity.server.host"), config.Configuration.Int("identity.server.ssh.port"))),
				wish.WithHostKeyPath(HostKeyPath),
			)
			if err != nil {
				log.Error("could not start server", "error", err)
				return
			}

			metrics := promwish.NewServer(
				"localhost:9222",
				promhttp.InstrumentMetricHandler(
					registry, promhttp.HandlerFor(registry, promhttp.HandlerOpts{}),
				),
			)

			done := make(chan os.Signal, 1)
			go func() {
				log.Info("Starting ssh server", "identity.server.host", config.Configuration.String("identity.server.host"), "identity.server.ssh.port", config.Configuration.Int("identity.server.ssh.port"), "address", fmt.Sprintf("%s:%d", config.Configuration.String("identity.server.host"), config.Configuration.Int("identity.server.ssh.port")))
				if err := s.ListenAndServe(); err != nil && !errors.Is(err, ssh.ErrServerClosed) {
					log.Error("could not start server", "error", err)
					done <- os.Interrupt
				}
			}()
			go func() {
				log.Info("Starting metrics server", "address", "localhost:9222")
				if err = metrics.ListenAndServe(); err != nil {
					log.Fatal("Fail to start metrics server", "error", err)
					done <- os.Interrupt
				}
			}()

			go func() {
				select {
				case <-goctx.Done():
					done <- os.Interrupt
				case <-done:
				}
			}()

			signal.Notify(done, os.Interrupt, syscall.SIGINT, syscall.SIGTERM)
			<-done
			log.Info("Done signal received, shutting down ssh server and metrics server.")
			ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
			defer cancel()
			log.Info("Shutting down ssh server", "identity.server.host", config.Configuration.String("identity.server.host"), "identity.server.ssh.port", config.Configuration.Int("identity.server.ssh.port"), "address", fmt.Sprintf("%s:%d", config.Configuration.String("identity.server.host"), config.Configuration.Int("identity.server.ssh.port")))
			if err := s.Shutdown(ctx); err != nil && !errors.Is(err, ssh.ErrServerClosed) {
				log.Error("could not stop server", "error", err)
			}
			log.Info("Stopping metrics server", "address", "localhost:9222")
			if err := metrics.Shutdown(ctx); err != nil && !errors.Is(err, ssh.ErrServerClosed) {
				log.Error("could not stop metrics server", "error", err)
			}
			log.Info("Stopped ssh and metrics servers", "time", time.Now())
		}
	}
}

type errMsg error

type model struct {
	ready                bool
	content              string
	viewport             viewport.Model
	spinner              spinner.Model
	quitting             bool
	err                  error
	term                 string
	width                int
	height               int
	meltedPrivateKeySeed string
	choices              []string
	cursor               int
	selected             map[int]struct{}
	charmId              string
	publicKeyAuthorized  string
}

var quitKeys = key.NewBinding(
	key.WithKeys("q", "esc", "ctrl+c"),
	key.WithHelp("", "press q to quit"),
)

func (m model) Init() tea.Cmd {
	return m.spinner.Tick
}

const UseHighPerformanceRenderer = false

var (
	TitleStyle = func() lipgloss.Style {
		b := lipgloss.RoundedBorder()
		b.Right = "├"
		return lipgloss.NewStyle().BorderStyle(b).Padding(0, 1)
	}()

	InfoStyle = func() lipgloss.Style {
		b := lipgloss.RoundedBorder()
		b.Left = "┤"
		return TitleStyle.Copy().BorderStyle(b)
	}()
)

func (m model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	s := "Your term is %s\n"
	s += "Your window size is x: %d y: %d\n\n"

	s = fmt.Sprintf(s, m.term, m.width, m.height)

	s += "Which room?\n\n"

	for i, choice := range m.choices {

		// Is the cursor pointing at this choice?
		cursor := " " // no cursor
		if m.cursor == i {
			cursor = ">" // cursor!
		}

		// Is this choice selected?
		checked := " " // not selected
		if _, ok := m.selected[i]; ok {
			checked = "x" // selected!
		}

		s += fmt.Sprintf("%s [%s] %s\n", cursor, checked, choice)
	}
	s += "\n"

	if m.meltedPrivateKeySeed != "" {
		smelted := "Your private key seed is melted:\n\n%s\n\n"
		s += fmt.Sprintf(smelted, m.meltedPrivateKeySeed)
	} else {
		authorizedPublicKeyText := "Your authorized public key is:\n\n%s\n\n"
		s += fmt.Sprintf(authorizedPublicKeyText, m.publicKeyAuthorized)
	}
	charmIdText := "Your charm id is:\n\n%s\n\n"
	s += fmt.Sprintf(charmIdText, m.charmId)

	if m.err != nil {
		return m, tea.Quit
	}

	s += fmt.Sprintf("\n   %s Loading forever... %s\n\n", m.spinner.View(), quitKeys.Help().Desc)

	var wrapAt int
	maxWrapMargin := 24
	leastWrapColumnWithMargin := 24
	mostWrapColumnBeforeMaxWrapMargin := 228

	if m.width < leastWrapColumnWithMargin {
		wrapAt = m.width
		s = wrap.String(s, wrapAt)
	} else {
		var wrapAt int
		if m.width <= mostWrapColumnBeforeMaxWrapMargin {
			wrapAt = m.width - int(1+((m.width-(leastWrapColumnWithMargin+1))*maxWrapMargin)/(mostWrapColumnBeforeMaxWrapMargin-(leastWrapColumnWithMargin+1)))
		} else {
			wrapAt = m.width - (maxWrapMargin + 1)
		}
		s = wordwrap.String(s, wrapAt)
	}
	s = wrap.String(s, m.width)
	if m.quitting {
		return m, tea.Quit
	}
	m.viewport.SetContent(s)

	var (
		cmd  tea.Cmd
		cmds []tea.Cmd
	)
	switch msg := msg.(type) {

	case tea.KeyMsg: // todo: super broken, fix this
		if key.Matches(msg, quitKeys) {
			m.quitting = true
			return m, tea.Quit
		}

		switch msg.String() {
		// The "up" and "k" keys move the cursor up
		case "w", "k":
			if m.cursor > 0 {
				m.cursor--
			}

		// The "down" and "j" keys move the cursor down
		case "s", "j":
			if m.cursor < len(m.choices)-1 {
				m.cursor++
			}

		// The "enter" key and the spacebar (a literal space) toggle
		// the selected state for the item that the cursor is pointing at.
		case "enter", " ":
			_, ok := m.selected[m.cursor]
			if ok {
				delete(m.selected, m.cursor)
			} else {
				m.selected[m.cursor] = struct{}{}
			}
		}
	case tea.WindowSizeMsg:
		m.height = msg.Height
		m.width = msg.Width
		if !m.ready {
			m.viewport = viewport.New(msg.Width, msg.Height)
			m.viewport.KeyMap.Down.SetKeys("down")
			m.viewport.KeyMap.Up.SetKeys("up")
			m.ready = true
		} else {
			m.viewport.Width = msg.Width
			m.viewport.Height = msg.Height
		}
	case errMsg:
		m.err = msg
	default:
		m.spinner, cmd = m.spinner.Update(msg)
	}

	m.viewport, cmd = m.viewport.Update(msg)

	cmds = append(cmds, cmd)

	return m, tea.Batch(cmds...)
}

func (m model) View() string {
	return m.viewport.View()
}

func TeaHandler(s ssh.Session) (tea.Model, []tea.ProgramOption) {
	pty, _, active := s.Pty()
	if !active {
		wish.Fatalln(s, "no active terminal, skipping")
		return nil, nil
	}
	sp := spinner.New()
	sp.Spinner = spinner.Dot
	sp.Style = lipgloss.NewStyle().Foreground(lipgloss.Color("205"))
	meltedPrivateKeySeed := s.Context().Permissions().Extensions["private-key-seed-melted"]
	m := model{
		spinner:              sp,
		quitting:             false,
		err:                  nil,
		term:                 pty.Term,
		width:                pty.Window.Width,
		height:               pty.Window.Height,
		meltedPrivateKeySeed: meltedPrivateKeySeed,
		choices:              []string{"Chat", "Game", "Upload"},
		selected:             make(map[int]struct{}),
		charmId:              s.Context().Permissions().Extensions["charm-id"],
		publicKeyAuthorized:  s.Context().Permissions().Extensions["public-key-authorized"],
	}
	return m, []tea.ProgramOption{tea.WithAltScreen()}
}

func Banner(config *configuration.IdentityServerConfiguration) func(ctx ssh.Context) string {
	return func(ctx ssh.Context) string {
		return `
Welcome to the identity server! ("The Service")

By using The Service, you agree to all of the following terms and conditions.

The user expressly understands and agrees that developing.today LLC, the operator of The Service, shall not be liable, in law or in equity, to them or to any third party for any direct, indirect, incidental, lost profits, special, consequential, punitive or exemplary damages.

EACH PARTY MAKES NO WARRANTIES, EXPRESS, IMPLIED OR OTHERWISE, REGARDING ACCURACY, COMPLETENESS OR PERFORMANCE.

THE SERVICE AND ANY RELATED SERVICES ARE PROVIDED ON AN "AS IS" AND "AS AVAILABLE" BASIS, WITHOUT WARRANTY OF ANY KIND, WHETHER WRITTEN OR ORAL, EXPRESS OR IMPLIED.

TO THE FULL EXTENT PERMISSIBLE BY LAW, DEVELOPING.TODAY LLC WILL NOT BE LIABLE FOR ANY DAMAGES OF ANY KIND ARISING FROM THE USE OF ANY DEVELOPING.TODAY LLC SERVICE, OR FROM ANY INFORMATION, CONTENT, MATERIALS, PRODUCTS (INCLUDING SOFTWARE) OR OTHER SERVICES INCLUDED ON OR OTHERWISE MADE AVAILABLE TO YOU THROUGH ANY DEVELOPING.TODAY LLC SERVICE, INCLUDING, BUT NOT LIMITED TO DIRECT, INDIRECT, INCIDENTAL, PUNITIVE, AND CONSEQUENTIAL DAMAGES, UNLESS OTHERWISE SPECIFIED IN WRITING.

TO THE MAXIMUM EXTENT ALLOWED BY LAW, DEVELOPING.TODAY LLC DISCLAIMS ALL WARRANTIES AND REPRESENTATIONS OF ANY KIND, INCLUDING WITHOUT LIMITATION THE IMPLIED WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE, AND NONINFRINGEMENT, WHETHER EXPRESS, IMPLIED, OR STATUTORY. DEVELOPING.TODAY LLC PROVIDES NO GUARANTEES THAT THE SERVICES OR NETWORK WILL FUNCTION WITHOUT INTERRUPTION OR ERRORS AND PROVIDES THE NETWORK, SERVICES, AND ANY RELATED CONTENT OR PRODUCTS SUBJECT TO THESE PUBLIC NETWORK TERMS ON AN “AS IS” BASIS.

By submitting your content (all information you transmit to any developing.today LLC service) ("Your Content") you hereby grant to developing.today LLC an irrevocable, perpetual, royalty-free, worldwide right and license (with right to sublicense) to use, distribute, reproduce, create derivate works of, perform and display Your Content, in whole or part, on or off the any developing.today LLC service for any purpose, commercial or otherwise without acknowledgment, consent or monetary or other compensation to you.

You hereby represent and warrant that:
- Your Content is an original work created by you
- You have all the rights and consents in and to Your Content necessary to grant the above license
- Your Content does not violate any privacy, publicity or any other applicable laws or regulations
- You understand and agree that developing.today LLC may use any information provided on this form or information available that is associated with your developing.today LLC account to contact you about Your Content.
- You agree that developing.today LLC has no obligation to exercise or exploit the above license.

If you do not agree to all of the above terms and conditions, then you may not use The Service and must disconnect immediately.
` + fmt.Sprintf("You are using the identity server at %s:%d\n", config.Configuration.String("identity.server.host"), config.Configuration.Int("identity.server.ssh.port")) + `
` + fmt.Sprintf("You are connecting from %s\n", ctx.RemoteAddr().String()) + `
` + fmt.Sprintf("You are connecting from-with %s\n", ctx.RemoteAddr().Network()) + `
` + fmt.Sprintf("You are connecting to %s\n", ctx.LocalAddr().String()) + `
` + fmt.Sprintf("You are connecting to-with %s\n", ctx.LocalAddr().Network()) + `
` + fmt.Sprintf("Your server version is %s\n", ctx.ServerVersion()) + `
` + fmt.Sprintf("Your client version is %s\n", ctx.ClientVersion()) + `
` + fmt.Sprintf("Your session id is %s\n", ctx.SessionID()) + `
` + fmt.Sprintf("You are connecting with user %s\n", ctx.User())
	}
}

type Challenge struct {
	Name        string
	Instruction string
	Questions   []Question
}

type Question struct {
	Question   string
	Answer     string
	HideAnswer bool
}

func (c Challenge) ExecuteMutable(challenge gossh.KeyboardInteractiveChallenge) ([]string, error) {
	var questions []string
	var showAnswers []bool
	for _, question := range c.Questions {
		questions = append(questions, question.Question)
		showAnswers = append(showAnswers, !question.HideAnswer)
	}
	answers, err := challenge(c.Name, c.Instruction, questions, showAnswers)
	if err != nil {
		return nil, err
	}
	for i, answer := range answers {
		c.Questions[i].Answer = answer
	}
	return answers, nil
}

func Connect(ctx ssh.Context, key ssh.PublicKey, password *string, challenge gossh.KeyboardInteractiveChallenge, connections *auth.SafeConnectionMap) bool {
	status := "open"
	app := "identity"
	connectionType := "ssh"
	user := ctx.User()
	var authMethod string

	if key != nil {
		authMethod = "public-key"
	} else if password != nil {
		authMethod = "password"
	} else if challenge != nil {
		authMethod = "keyboard-interactive"
	} else {
		log.Error("No authentication method provided")
		return false
	}

	if ctx.Permissions().Extensions == nil {
		ctx.Permissions().Extensions = make(map[string]string)
	}

	var interactive *string

	if challenge != nil {
		c := Challenge{
			Name:        "Room Challenge:",
			Instruction: "Select your room and enter the password if required.",
			Questions: []Question{
				{
					Question: "What is the room? ",
					Answer:   "",
				},
				{
					Question:   "What is the password? (leave blank if none, password is sometimes required. Passwords are insecure, passwords may be visible to others.) ",
					Answer:     "",
					HideAnswer: true,
				},
			},
		}
		_, err := c.ExecuteMutable(challenge)
		if err != nil {
			log.Error("Failed to get keyboard interactive response", "error", err)
			return false
		}
		ctx.Permissions().Extensions["room"] = c.Questions[0].Answer
		password = &c.Questions[1].Answer

		challengesJson, err := json.Marshal(c)
		if err != nil {
			log.Error("Failed to marshal challenges", "error", err)
			return false
		}
		interactiveStr := string(challengesJson)
		interactive = &interactiveStr

		log.Info("Accepting keyboard interactive", "response", interactiveStr, "len", len(interactiveStr))
	}

	var passwordLength *int64
	var passwordHash *string
	var passwordHashType *string
	var passwordSha256 []byte
	var passwordSha256Str string

	if password != nil {
		log.Info("Accepting password", "password", *password, "len", len(*password))
		passwordLength = new(int64)
		*passwordLength = int64(len(*password))
		hasher := sha256.New()
		hasher.Write([]byte(*password))
		passwordSha256 = hasher.Sum(nil)
		passwordSha256Str = base64.StdEncoding.EncodeToString(passwordSha256)
		passwordHash = &passwordSha256Str
		ctx.Permissions().Extensions["password-hash"] = *passwordHash

		passwordHashTypeStr := "sha256"
		passwordHashType = &passwordHashTypeStr
		ctx.Permissions().Extensions["password-hash-type"] = *passwordHashType

		log.Info("Accepting password", "passwordHash", *passwordHash)
	}

	var publicKey *string
	var publicKeyType string

	if key != nil {
		log.Info("Accepting public key", "publicKeyType", key.Type(), "publicKeyString", base64.StdEncoding.EncodeToString(key.Marshal()))
		publicKeyStr := base64.StdEncoding.EncodeToString(key.Marshal())
		publicKey = &publicKeyStr
		publicKeyType = key.Type()
		ctx.Permissions().Extensions["public-key"] = *publicKey
		ctx.Permissions().Extensions["public-key-type"] = publicKeyType
	}
	var textKeyId *int64
	var hashKeyId *int64
	var ed25519PrivateKey ed25519.PrivateKey
	var ed25519PublicKey ed25519.PublicKey
	var privateKeyId *int64

	if publicKey == nil {
		log.Info("No public key provided, gathering one")
		if password == nil || passwordLength == nil || passwordHash == nil || passwordHashType == nil || passwordSha256 == nil {
			log.Error("No public key or password provided", "password", *password, "passwordLength", *passwordLength, "passwordHash", *passwordHash, "passwordHashType", *passwordHashType, "passwordSha256", passwordSha256)
			return false
		}

		if interactive != nil {
			publicKeyStr, err := auth.GetPublicKeyFromText(passwordSha256Str, "%")
			if err != nil {
				log.Info("Failed to get public key from text", "error", err)
			} else {
				log.Info("Got public key from text", "publicKeyStr", publicKeyStr)
				if publicKeyStr != "" {
					out, comment, options, rest, err := gossh.ParseAuthorizedKey([]byte(publicKeyStr))
					if err != nil {
						log.Error("Failed to parse public key", "error", err)
						return false
					}
					log.Info("Parsed public key", "out", out, "comment", comment, "options", options, "rest", rest)
					publicKey = &publicKeyStr
					publicKeyType = out.Type()

					key = out
					log.Info("Gathered public key", "publicKey", publicKeyStr)
					ctx.Permissions().Extensions["public-key-type"] = publicKeyType
					ctx.Permissions().Extensions["public-key-authorized"] = publicKeyStr
					log.Info("Setting permissions extensions", "public-key-type", publicKeyType, "public-key-authorized", publicKeyStr, "public-key", publicKeyStr, "public-key-type", publicKeyType)
				}
			}
		} else {
			publicKeyStr, err := auth.GetPublicKeyFromHash(passwordSha256Str, "%")
			if err != nil {
				log.Info("Failed to get public key from hash", "error", err)
			} else {
				log.Info("Got public key from hash", "publicKeyStr", publicKeyStr)
				if publicKeyStr != "" {
					out, comment, options, rest, err := gossh.ParseAuthorizedKey([]byte(publicKeyStr))
					if err != nil {
						log.Error("Failed to parse public key", "error", err)
						return false
					}
					log.Info("Parsed public key", "out", out, "comment", comment, "options", options, "rest", rest)
					publicKey = &publicKeyStr
					publicKeyType = out.Type()

					key = out
					log.Info("Gathered public key", "publicKey", publicKeyStr)
					ctx.Permissions().Extensions["public-key-type"] = publicKeyType
					ctx.Permissions().Extensions["public-key-authorized"] = publicKeyStr
					log.Info("Setting permissions extensions", "public-key-type", publicKeyType, "public-key-authorized", publicKeyStr, "public-key", publicKeyStr, "public-key-type", publicKeyType)
				}
			}
		}
		if key == nil {
			log.Info("No public key found, generating one")
			if interactive != nil {
				ed25519PrivateKey = ed25519.NewKeyFromSeed(passwordSha256)
				ed25519PublicKey = ed25519PrivateKey.Public().(ed25519.PublicKey)
			} else {
				var err error
				ed25519PublicKey, ed25519PrivateKey, err = ed25519.GenerateKey(nil)
				if err != nil {
					log.Error("Failed to generate private key", "error", err)
					return false
				}
			}
			log.Info("Generated private key", "pk", ed25519PrivateKey, "pkLen", len(ed25519PrivateKey), "pkStr", base64.StdEncoding.EncodeToString(ed25519PrivateKey))

			privateKeyIdi, err := auth.InsertPrivateKey(ed25519PrivateKey)
			if err != nil {
				log.Error("Failed to insert private key", "error", err)
				return false
			}
			privateKeyId = &privateKeyIdi

			log.Info("Generated public key", "pk", ed25519PublicKey, "pkLen", len(ed25519PublicKey), "pkStr", base64.StdEncoding.EncodeToString(ed25519PublicKey), "privateKeyId", *privateKeyId)
			ctx.Permissions().Extensions["private-key-seed"] = base64.StdEncoding.EncodeToString(ed25519PrivateKey.Seed())
			ctx.Permissions().Extensions["private-key"] = base64.StdEncoding.EncodeToString(ed25519PrivateKey)
			ctx.Permissions().Extensions["private-key-type"] = "ed25519"
			ctx.Permissions().Extensions["public-key"] = base64.StdEncoding.EncodeToString(ed25519PublicKey)
			ctx.Permissions().Extensions["public-key-type"] = "ed25519"

			sshPubKey, err := gossh.NewPublicKey(ed25519PublicKey)
			if err != nil {
				log.Fatal("Failed to create SSH public key", err)
			}

			if interactive != nil {
				textKeyIdi, err := auth.InsertTextPublicKey(passwordSha256Str, "sha256", sshPubKey)
				if err != nil {
					log.Error("Failed to insert text public key", "error", err)
					return false
				}
				textKeyId = &textKeyIdi
				log.Info("Inserted text public key", "textKeyId", *textKeyId)
			} else {
				hashKeyIdi, err := auth.InsertHashPublicKey(passwordSha256Str, "sha256", sshPubKey)
				if err != nil {
					log.Error("Failed to insert hash public key", "error", err)
					return false
				}
				hashKeyId = &hashKeyIdi
				log.Info("Inserted hash public key", "hashKeyId", *hashKeyId)
			}

			authorizedKey := gossh.MarshalAuthorizedKey(sshPubKey)
			authKey := string(authorizedKey)
			log.Info("Generated public key", "authKey", authKey, "authorizedKey", authorizedKey, "sshPubKey", sshPubKey, "sshPubKeyStr", string(sshPubKey.Marshal()))
			ctx.Permissions().Extensions["public-key-authorized"] = authKey

			publicKeyStr := base64.StdEncoding.EncodeToString(authorizedKey)
			log.Info("Generated public key", "publicKeyStr", publicKeyStr)

			publicKey = &publicKeyStr
			publicKeyType = "ed25519"
			log.Info("Generated public key", "publicKey", *publicKey)
			parts := strings.Fields(string(authorizedKey))
			if len(parts) < 2 {
				log.Fatal("Invalid public key format")
			}
			keyData, err := base64.StdEncoding.DecodeString(parts[1])
			if err != nil {
				log.Fatal("Failed to decode base64 public key", err)
			}
			log.Info("Generated public key, preparing", "keyData", keyData, "keyDataLen", len(keyData), "parts", parts, "publicKey", *publicKey)

			out, comment, options, rest, err := gossh.ParseAuthorizedKey(authorizedKey)
			if err != nil {
				log.Fatal("Failed to parse public key", "error", err)
			}
			log.Info("Parsed public key", "out", out, "comment", comment, "options", options, "rest", rest)
			key = out
			log.Info("Generated public key", "publicKey", publicKeyStr)
			ctx.Permissions().Extensions["public-key"] = *publicKey
			ctx.Permissions().Extensions["public-key-type"] = publicKeyType
			pkMelted, err := melt.ToMnemonic(&ed25519PrivateKey)
			if err != nil {
				log.Error("Failed to melt private key", "error", err)
				return false
			}
			ctx.Permissions().Extensions["private-key-seed-melted"] = pkMelted
			log.Info("Melted private key", "pkMelted", pkMelted)
		}
	} else {
		log.Info("Public key provided", "publicKey", *publicKey, "key", key, "keyType", key.Type(), "keyMarshal", key.Marshal(), "keyMarshalLen", len(key.Marshal()))
	}

	if publicKey == nil {
		log.Error("No public key provided")
		return false
	}

	authorizedKey := gossh.MarshalAuthorizedKey(key)
	log.Info("Public key used", "publicKey", authorizedKey)

	serverVersion := ctx.ServerVersion()
	clientVersion := ctx.ClientVersion()
	sessionHash := ctx.SessionID()
	permissionsCriticalOptionsJson, err := json.Marshal(ctx.Permissions().CriticalOptions)
	if err != nil {
		log.Error("Failed to marshal critical options", "error", err)
		return false
	}
	permissionsCriticalOptions := string(permissionsCriticalOptionsJson)
	host := ctx.LocalAddr().String()
	port := int64(ctx.LocalAddr().(*net.TCPAddr).Port)
	remoteAddr := ctx.RemoteAddr().String()
	remoteAddrNetwork := ctx.RemoteAddr().Network()
	openedAt := time.Now()
	pty := ""
	protocol := "ssh"
	permissionsExtensions := ""
	admin := ""
	query := ""
	commands := ""
	comments := ""
	history := ""

	log.Info("Connection opened", "openedAt", openedAt, "remoteAddr", remoteAddr, "remoteAddrNetwork", remoteAddrNetwork, "host", host, "port", port, "serverVersion", serverVersion, "clientVersion", clientVersion, "sessionHash", sessionHash, "permissionsCriticalOptions", permissionsCriticalOptions)

	interactiveStr := ""
	if interactive != nil {
		interactiveStr = *interactive
	}

	connection := auth.Connection{
		Status:                     &status,
		Name:                       &user,
		Description:                &user,
		App:                        &app,
		AuthMethod:                 &authMethod,
		Type:                       &connectionType,
		Username:                   &user,
		PublicKey:                  publicKey,
		ServerVersion:              &serverVersion,
		ClientVersion:              &clientVersion,
		SessionHash:                &sessionHash,
		PermissionsCriticalOptions: &permissionsCriticalOptions,
		PermissionsExtensions:      &permissionsExtensions,
		Host:                       &host,
		Port:                       port,
		Pty:                        &pty,
		Protocol:                   &protocol,
		RemoteAddr:                 &remoteAddr,
		RemoteAddrNetwork:          &remoteAddrNetwork,
		OpenedAt:                   &openedAt,
		Interactive:                &interactiveStr,
		PasswordLength:             passwordLength,
		PasswordHash:               passwordHash,
		PasswordHashType:           passwordHashType,
		Admin:                      &admin,
		Query:                      &query,
		Commands:                   &commands,
		Comments:                   &comments,
		History:                    &history,
	}

	log.Info("Inserting connection", "connection", connection.ToData(), "connectionID", connection.ConnectionID)
	connectionID, err := connection.Insert()

	if err != nil {
		log.Error("Failed to insert connection", "error", err, "connectionID", connection.ConnectionID)
		return false
	}
	log.Info("Inserted connection", "connectionID", &connectionID, "connection", connection.String(), "connectionID", connection.ConnectionID)
	ctx.Permissions().Extensions["connection-id"] = *connectionID

	permissionsExtensionsJson, err := json.Marshal(ctx.Permissions().Extensions)
	if err != nil {
		log.Error("Failed to marshal extensions", "error", err, "connectionID", connection.ConnectionID)
		return false
	}
	log.Info("Setting permissions extensions", "permissionsExtensions", string(permissionsExtensionsJson), "connectionID", connection.ConnectionID)
	connection.SetPermissionsExtensions(string(permissionsExtensionsJson))

	log.Info("Checking public key", "publicKey", *publicKey, "connectionID", connection.ConnectionID)
	result, err := auth.CheckPublicKey(ctx, key)

	log.Info("Checked public key", "result", result, "error", err, "connectionID", connection.ConnectionID)
	if err != nil {
		var userID int64
		userID, err = auth.InsertUser(ctx)
		if err != nil {
			log.Error("Failed to insert user", "error", err, "connectionID", connection.ConnectionID)
			return false
		}
		log.Info("Inserted user", "userID", userID, "connectionID", connection.ConnectionID)

		var pk int64
		pk, err = auth.InsertPublicKey(userID, key)
		if err != nil {
			log.Error("Failed to insert public key", "error", err, "connectionID", connection.ConnectionID)
			return false
		}
		log.Info("Inserted public key", "pk", pk, "connectionID", connection.ConnectionID)

		result, err = auth.CheckPublicKey(ctx, key)

		log.Info("Checked public key", "result", result, "error", err, "connectionID", connection.ConnectionID)
	} else {
		log.Info("Public key already exists", "result", result, "connectionID", connection.ConnectionID)
	}
	if err != nil {
		log.Error("Failed to check public key", "error", err, "connectionID", connection.ConnectionID)
		return false
	}
	connection.SetCharmID(result.ID)
	if ed25519PrivateKey != nil {
		affected, err := auth.UpdatePrivateKey(*privateKeyId, &result.ID, connectionID)
		if err != nil {
			log.Error("Failed to update private key", "error", err, "connectionID", connection.ConnectionID)
			return false
		}
		log.Info("Updated private key", "affected", affected, "connectionID", connection.ConnectionID)
		if affected < 1 {
			log.Error("Failed to update private key, affected 0", "error", err, "connectionID", connection.ConnectionID)
			return false
		}
	}
	if textKeyId != nil {
		affected, err := auth.UpdateTextPublicKey(*textKeyId, &result.ID, connectionID)
		if err != nil {
			log.Error("Failed to update text public key", "error", err, "connectionID", connection.ConnectionID)
			return false
		}
		log.Info("Updated text public key", "affected", affected, "connectionID", connection.ConnectionID)

		if affected < 1 {
			log.Error("Failed to update text public key, affected 0", "error", err, "connectionID", connection.ConnectionID)
			return false
		}
	}
	if hashKeyId != nil {
		affected, err := auth.UpdateHashPublicKey(*hashKeyId, &result.ID, connectionID)
		if err != nil {
			log.Error("Failed to update hash public key", "error", err, "connectionID", connection.ConnectionID)
			return false
		}
		log.Info("Updated hash public key", "affected", affected, "connectionID", connection.ConnectionID)
		if affected < 1 {
			log.Error("Failed to update hash public key, affected 0", "error", err, "connectionID", connection.ConnectionID)
			return false
		}
	}
	ctx.Permissions().Extensions["charm-id"] = result.ID
	connections.Set(*connection.ConnectionID, &connection)
	ctx.SetValue("connection", connection)
	ctx.Permissions().Extensions["charm-name"] = result.Name
	log.Info("Setting permissions extensions", "charm-id", result.ID, "charm-name", result.Name, "connectionID", connection.ConnectionID)
	jsonRoles, err := json.Marshal(result.Roles)
	if err != nil {
		log.Error("Failed to marshal roles", "error", err)
		return false
	}
	log.Info("Setting permissions extensions", "charm-roles", string(jsonRoles))
	ctx.Permissions().Extensions["charm-roles"] = string(jsonRoles)
	ctx.Permissions().Extensions["charm-created-at"] = result.CreatedAt.Format(time.RFC3339)
	ctx.Permissions().Extensions["charm-public-key-created-at"] = result.PublicKeyCreatedAt.Format(time.RFC3339)
	ctx.Permissions().Extensions["charm-public-key-type"] = result.PublicKeyType
	ctx.Permissions().Extensions["charm-public-key"] = result.PublicKeyString

	log.Info("Setting permissions extensions", "charm-created-at", result.CreatedAt.Format(time.RFC3339), "charm-public-key-created-at", result.PublicKeyCreatedAt.Format(time.RFC3339), "charm-public-key-type", result.PublicKeyType, "charm-public-key", result.PublicKeyString)

	return true
}
