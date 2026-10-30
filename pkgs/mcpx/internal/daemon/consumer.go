package daemon

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/dezren39/mcpx/internal/codegen"
	"github.com/dezren39/mcpx/internal/config"
	"github.com/dezren39/mcpx/internal/defaults"
	"github.com/dezren39/mcpx/internal/diagnose"
	"github.com/dezren39/mcpx/internal/elicit"
	"github.com/dezren39/mcpx/internal/events"
	"github.com/dezren39/mcpx/internal/mcpclient"
	"github.com/dezren39/mcpx/internal/model"
	"github.com/dezren39/mcpx/internal/pool"
	"github.com/dezren39/mcpx/internal/recipes"
	"github.com/dezren39/mcpx/internal/runner"
	"github.com/dezren39/mcpx/internal/scripting"
	"github.com/dezren39/mcpx/internal/searchpath"
	"github.com/dezren39/mcpx/internal/settings"
)

// ConsumerPolicy is the resolved policy for everything mcpx asks for on its
// own behalf rather than forwarding.
//
// Resolved once, in the daemon, from the same configuration files and
// variables the CLI reads. Whether a destructive call is confirmed is not a
// question with two correct answers depending on which process asked, so the
// answer is computed where the call happens.
type ConsumerPolicy struct {
	Disambiguate        string
	DisambiguateDefault string
	ConfirmDestructive  bool
	AskTimeout          time.Duration
	DiagnosePreflight   bool
	HistoryPerTool      int
	RecipeMinScore      int
	RecipeMatchMargin   int
	RecipeLimit         int
	PromptSample        string
	PromptCatalogBudget int
	PromptSampleTimeout time.Duration
	PromptMaxTokens     int
	RunTimeout          time.Duration
	ModelProvider       string
	ModelBaseURL        string
	ModelAPIKeyEnv      string
	ModelDefault        string
	ModelBudget         int
	ModelTimeout        time.Duration
}

// consumerState is everything the consumer routes need that the daemon did
// not already hold.
type consumerState struct {
	policy  ConsumerPolicy
	history *diagnose.History
	// dirs is the recipe search path, nearest first.
	dirs []string
}

// CatalogHistoryFile is the name of the file that remembers what every tool's
// schema used to look like.
const CatalogHistoryFile = "catalog-history.json"

// initConsumer resolves the policy and opens the schema history.
//
// Failures here are not fatal. A daemon with no policy file still serves
// tools, and a diagnostic that cannot say when a schema changed is still a
// diagnostic that says what changed.
func (s *Server) initConsumer() {
	st := &consumerState{policy: defaultConsumerPolicy()}
	var sources []string
	if s.cfg != nil {
		sources = s.cfg.Sources
	}
	if set, err := settings.Resolve(sources); err != nil {
		s.logger.Printf("consumer policy: %v; using defaults", err)
	} else {
		st.policy = consumerPolicyFrom(set)
		st.dirs = recipeDirs(set.String("paths.scripts"))
	}
	if len(st.dirs) == 0 {
		st.dirs = recipeDirs("")
	}
	st.history = diagnose.OpenHistory(
		filepath.Join(s.paths.State, CatalogHistoryFile), st.policy.HistoryPerTool)

	s.consumer = st
	s.reg.consumer = st.policy
	s.reg.history = st.history
	// The cache the registry loaded at startup is an observation like any
	// other, and recording it now is what makes the first refresh after a
	// restart a diff rather than a first sighting.
	s.reg.ObserveCatalog()
}

func defaultConsumerPolicy() ConsumerPolicy {
	return ConsumerPolicy{
		Disambiguate:        defaults.Disambiguate,
		DisambiguateDefault: defaults.DisambiguateDefault,
		ConfirmDestructive:  defaults.ConfirmDestructive,
		AskTimeout:          defaults.AskTimeout,
		DiagnosePreflight:   defaults.DiagnosePreflight,
		HistoryPerTool:      defaults.HistoryPerTool,
		RecipeMinScore:      defaults.RecipeMinScore,
		RecipeMatchMargin:   defaults.RecipeMatchMargin,
		RecipeLimit:         defaults.RecipeLimit,
		PromptSample:        defaults.PromptSample,
		PromptCatalogBudget: defaults.PromptCatalogBudget,
		PromptSampleTimeout: defaults.PromptSampleTimeout,
		PromptMaxTokens:     defaults.PromptMaxTokens,
		RunTimeout:          defaults.RunTimeout,
	}
}

func consumerPolicyFrom(set *settings.Set) ConsumerPolicy {
	p := defaultConsumerPolicy()
	p.Disambiguate = set.String("elicit.disambiguate")
	p.DisambiguateDefault = set.String("elicit.disambiguateDefault")
	p.ConfirmDestructive = set.Bool("elicit.confirmDestructive")
	p.AskTimeout = set.Duration("elicit.askTimeout")
	p.DiagnosePreflight = set.Bool("diagnose.preflight")
	p.HistoryPerTool = set.Int("diagnose.history")
	p.RecipeMinScore = set.Int("recipes.minScore")
	p.RecipeMatchMargin = set.Int("recipes.matchMargin")
	p.RecipeLimit = set.Int("recipes.limit")
	p.PromptSample = set.String("prompt.sample")
	p.PromptCatalogBudget = set.Int("prompt.catalogBudget")
	p.PromptSampleTimeout = set.Duration("prompt.sampleTimeout")
	p.PromptMaxTokens = set.Int("prompt.maxTokens")
	p.RunTimeout = set.Duration("prompt.runTimeout")
	p.ModelProvider = set.String("model.provider")
	p.ModelBaseURL = set.String("model.baseURL")
	p.ModelAPIKeyEnv = set.String("model.apiKeyEnv")
	p.ModelDefault = set.String("model.default")
	p.ModelBudget = set.Int("model.budget")
	p.ModelTimeout = set.Duration("model.timeout")
	return p
}

// recipeDirs resolves the script search path from the daemon's own working
// directory, which is the project it was started for.
func recipeDirs(configured string) []string {
	wd, _ := os.Getwd()
	home, _ := os.UserHomeDir()
	builtin, _ := recipes.Dirs(wd, home, os.Getenv("XDG_CONFIG_HOME"))
	list := splitPathList(configured)
	if len(list) == 0 {
		return builtin
	}
	resolved := searchpath.Resolve(list, settings.NullMarker, searchpath.Options{
		Dir: wd, Builtin: builtin,
	})
	var out []string
	for _, e := range resolved.Entries {
		out = append(out, e.Path)
	}
	return out
}

// splitPathList accepts a JSON array as a config file holds it, or a
// separator-joined string as a variable must.
func splitPathList(v string) []string {
	v = strings.TrimSpace(v)
	if v == "" {
		return nil
	}
	if strings.HasPrefix(v, "[") {
		var arr []any
		if json.Unmarshal([]byte(v), &arr) == nil {
			var parts []string
			for _, e := range arr {
				if e == nil {
					parts = append(parts, settings.NullMarker)
					continue
				}
				parts = append(parts, fmt.Sprint(e))
			}
			return parts
		}
	}
	var parts []string
	for _, part := range strings.Split(v, string(os.PathListSeparator)) {
		switch part = strings.TrimSpace(part); part {
		case "":
		case "-", "null":
			parts = append(parts, settings.NullMarker)
		default:
			parts = append(parts, part)
		}
	}
	return parts
}

// ---- the catalog's own history ----

// DiagnoseTools is every visible tool, flattened for the diagnostics.
func (r *Registry) DiagnoseTools() []diagnose.Tool {
	r.mu.RLock()
	defer r.mu.RUnlock()
	var out []diagnose.Tool
	if len(r.testTools) > 0 {
		for _, t := range r.testTools {
			fn := t.Function
			if fn == "" {
				fn = codegen.ToolFuncName(t.Tool)
			}
			out = append(out, diagnose.Tool{
				Namespace: t.Namespace,
				Name:      t.Tool,
				Func:      fn,
				Shape:     diagnose.ShapeOf(t.InputSchema),
			})
		}
		return out
	}
	for _, name := range r.order {
		p := r.pools[name]
		view := r.views[name]
		all, _, at := p.CachedSchemas()
		if at.IsZero() {
			// Nothing has been read from this server yet. Reporting no tools
			// would let the history record every one of them as removed.
			continue
		}
		for _, t := range visibleTools(view, all) {
			out = append(out, diagnose.Tool{
				Namespace: view.Namespace,
				Name:      t.Name,
				Func:      codegen.ToolFuncName(t.Name),
				Shape:     diagnose.ShapeOf(t.InputSchema),
			})
		}
	}
	return out
}

// ObserveCatalog records the current schemas and remembers what changed.
//
// This is the half of the deterministic diagnostic that did not exist.
// codegen could already fingerprint a catalog and diff two of them; nothing
// ever kept one, so the diff was always against nothing and the question
// "when did this change" had no answer.
func (r *Registry) ObserveCatalog() {
	if r.history == nil {
		return
	}
	changes := r.history.Observe(time.Now(), r.DiagnoseTools())
	if err := r.history.Save(); err != nil {
		r.logf("saving catalog history: %v", err)
	}
	for _, ch := range changes {
		r.logf("catalog: %s %s", ch.Tool, ch.What)
		r.publish(events.Event{
			Kind: events.CatalogChanged, Data: mustJSON(ch),
		})
	}
}

// DiagnoseCatalog is the view a diagnostic is computed against.
func (r *Registry) DiagnoseCatalog() diagnose.Catalog {
	cat := diagnose.Catalog{Tools: r.DiagnoseTools()}
	if r.history != nil {
		cat.Changes = r.history.All()
	}
	return cat
}

// ---- elicit to disambiguate ----

// disambiguate asks which live instance a call should use.
//
// It only ever asks when the answer is genuinely open: the server is
// exclusive (so instances are distinct things rather than copies), several
// are live, and the caller's own scope key names none of them. When the
// caller already has an instance, the scope key is the answer and a question
// would be theatre.
//
// The returned key is the one to lease. Every path returns something: an
// unanswered question falls back to the configured default, because the
// caller is frequently headless and a call that waits forever is worse than
// one that takes the conservative option.
func (r *Registry) disambiguate(ctx context.Context, p *pool.Pool, cc config.CallContext, key string) string {
	if r.consumer.Disambiguate != "ask" || r.broker == nil {
		return key
	}
	st := p.Status()
	if st.Sharing != string(config.SharingExclusive) || len(st.Instances) < 2 {
		return key
	}
	choices := []elicit.Choice{{
		Value: key, Title: "new",
		Detail: "start an instance for this caller, which is what happens today",
	}}
	recent := key
	bestIdle := -1
	for _, in := range st.Instances {
		if in.Key == key {
			// The caller already has one. That is the answer.
			return key
		}
		choices = append(choices, elicit.Choice{
			Value: in.Key, Title: in.Key, Detail: describeInstance(in),
		})
		if bestIdle < 0 || in.IdleSec < bestIdle {
			bestIdle, recent = in.IdleSec, in.Key
		}
	}
	sort.SliceStable(choices[1:], func(i, j int) bool {
		return choices[1+i].Title < choices[1+j].Title
	})

	fallback := key
	if r.consumer.DisambiguateDefault == "recent" {
		fallback = recent
	}
	req := elicit.Disambiguation(elicit.Ask{
		Server:  p.Name(),
		Session: cc.SessionID,
		Message: fmt.Sprintf("%s has %d live instances and this call named none of them. Which should it use?",
			p.Namespace(), len(st.Instances)),
		Choices: choices,
		Default: fallback,
		TTL:     r.consumer.AskTimeout,
	})
	opened, err := r.broker.OpenRequest(req)
	if err != nil {
		r.logf("disambiguate %s: %v", p.Name(), err)
		return fallback
	}
	r.publish(events.Event{Kind: events.ElicitOpened, Server: p.Name(), Data: mustJSON(elicitEventSummary(opened))})

	actx, cancel := context.WithTimeout(ctx, r.consumer.AskTimeout)
	defer cancel()
	ans, aerr := r.broker.Await(actx, opened.ID)
	if aerr != nil {
		r.logf("disambiguate %s: nobody answered, using %s", p.Name(), fallback)
		return fallback
	}
	chosen, answered := elicit.Resolve(ans, elicit.FieldInstance, fallback)
	if !answered {
		return fallback
	}
	// An answer naming something that is not on offer is a mistake, not an
	// instruction: honouring it would start a process keyed on arbitrary
	// text somebody typed.
	for _, c := range choices {
		if c.Value == chosen {
			return chosen
		}
	}
	r.logf("disambiguate %s: %q is not one of the choices, using %s", p.Name(), chosen, fallback)
	return fallback
}

func describeInstance(in pool.InstanceStatus) string {
	who := "idle"
	if in.Holders > 0 {
		who = fmt.Sprintf("held by %d caller(s)", in.Holders)
	}
	return fmt.Sprintf("%s, %d call(s), idle %ds, up %ds, pid %d",
		who, in.Calls, in.IdleSec, in.UptimeSec, in.PID)
}

// ---- confirm a destructive call ----

// ErrNotConfirmed is returned when a destructive call was not approved.
type ErrNotConfirmed struct {
	Tool   string
	Reason string
}

func (e ErrNotConfirmed) Error() string {
	return fmt.Sprintf("%s may be destructive and was not confirmed: %s", e.Tool, e.Reason)
}

// confirmDestructive asks before a call the server says cannot be undone.
//
// Off by default, and deliberately so: it puts a question on the path of
// every destructive call, and a caller with nobody to answer it waits out the
// deadline first. What it buys, when it is on, is the step a human-driven
// client has and a script does not -- the moment between deciding to delete
// fourteen issues and deleting them.
//
// An unanswered question refuses the call. That is the opposite of the
// disambiguation default, and for the opposite reason: there, doing nothing
// is safe; here, doing nothing is the destructive thing.
func (r *Registry) confirmDestructive(ctx context.Context, p *pool.Pool, tool string, cc config.CallContext) error {
	if !r.consumer.ConfirmDestructive || r.broker == nil {
		return nil
	}
	all, _, at := p.CachedSchemas()
	if at.IsZero() {
		// Never read, or invalidated by a list_changed. Deciding from an
		// empty or stale list would let the call through unasked -- which
		// is what happened on every call that arrived while a new daemon
		// was still reading schemas -- so read them now, and refuse if
		// they cannot be read: failing open is the one wrong answer here.
		fresh, _, err := p.RefreshSchemas(ctx)
		if err != nil {
			return ErrNotConfirmed{Tool: p.Namespace() + "." + tool,
				Reason: "its annotations could not be read: " + err.Error()}
		}
		all = fresh
	}
	var found *mcpclient.Tool
	for i := range all {
		if all[i].Name == tool {
			found = &all[i]
			break
		}
	}
	if found == nil || annotationClearsCall(p, found) {
		return nil
	}
	path := p.Namespace() + "." + tool
	req := elicit.Confirmation(elicit.Ask{
		Server:  p.Name(),
		Tool:    tool,
		Session: cc.SessionID,
		Message: fmt.Sprintf("%s.%s may be destructive: no trusted annotation clears this call. Let this call proceed?",
			p.Namespace(), tool),
		Default: "false",
		TTL:     r.consumer.AskTimeout,
	})
	opened, err := r.broker.OpenRequest(req)
	if err != nil {
		return err
	}
	r.publish(events.Event{Kind: events.ElicitOpened, Server: p.Name(), Data: mustJSON(elicitEventSummary(opened))})

	actx, cancel := context.WithTimeout(ctx, r.consumer.AskTimeout)
	defer cancel()
	ans, aerr := r.broker.Await(actx, opened.ID)
	if aerr != nil {
		return ErrNotConfirmed{Tool: path,
			Reason: "nobody answered within " + r.consumer.AskTimeout.String()}
	}
	if v, _ := elicit.Resolve(ans, elicit.FieldConfirm, "false"); v != "true" {
		return ErrNotConfirmed{Tool: path, Reason: "the answer was " + string(ans.Action)}
	}
	return nil
}

// annotationClearsCall reports whether a tool's annotations may clear
// its call without asking. Tool annotations are hints, not decisions
// (#2191): the specification forbids basing a security decision on
// annotations from an untrusted server, and mcpx establishes trust in
// exactly one way today -- the server is one the user declared locally,
// a stdio command from their own configuration, whose annotations are
// part of the interface they chose to run. A remote upstream's
// readOnlyHint or destructiveHint:false is worth nothing here: a lying
// or compromised server could suppress every confirmation by claiming
// its tools are harmless, so for a remote server the annotation can
// never be the deciding input and the call is confirmed when in doubt.
func annotationClearsCall(p *pool.Pool, found *mcpclient.Tool) bool {
	if p == nil || found == nil {
		return false
	}
	cfg := p.Config()
	if cfg == nil || !cfg.Stdio() {
		return false
	}
	return !destructiveHint(found.Annotations)
}

// destructiveHint reads the one annotation mcpx acts on, with the
// specification's defaults: destructiveHint is true when absent, and means
// something only when readOnlyHint is not true. An absent hint was read as
// false, so an unannotated tool -- which the schema says may destroy -- ran
// unconfirmed (#207, TOOL-17).
func destructiveHint(raw json.RawMessage) bool {
	var doc struct {
		ReadOnly    *bool `json:"readOnlyHint"`
		Destructive *bool `json:"destructiveHint"`
	}
	if len(raw) > 0 && json.Unmarshal(raw, &doc) != nil {
		// Unreadable annotations say nothing, so the default stands.
		return true
	}
	if doc.ReadOnly != nil && *doc.ReadOnly {
		return false
	}
	return doc.Destructive == nil || *doc.Destructive
}

// ---- recipes ----

// clientFileNames are files in a scripts directory that are machinery rather
// than something anybody would run.
func clientFileNames() map[string]bool {
	return map[string]bool{runner.ClientFileName: true, runner.GlobalsFileName: true}
}

func (s *Server) loadRecipes() []recipes.Recipe {
	if s.consumer == nil {
		return nil
	}
	return recipes.Load(s.consumer.dirs, clientFileNames())
}

func (s *Server) findRecipe(name string) (recipes.Recipe, bool) {
	for _, r := range s.loadRecipes() {
		if r.Name == name {
			return r, true
		}
	}
	return recipes.Recipe{}, false
}

// elicitPlaceholders asks for the values a recipe needs and did not get.
//
// One form for all of them rather than one question each: a person filling in
// four fields is doing one thing, and four questions with four deadlines is
// four chances for the run to die half-configured.
func (s *Server) elicitPlaceholders(ctx context.Context, r recipes.Recipe,
	missing []recipes.Placeholder, session string) (map[string]any, string, error) {

	if s.reg.broker == nil {
		return nil, "", fmt.Errorf("%s needs %s and there is nobody to ask: no question broker",
			r.Name, names(missing))
	}
	req := elicit.FormAsk(elicit.Ask{
		Session: session,
		Message: fmt.Sprintf("%s needs %s", r.Name, names(missing)),
		TTL:     s.consumer.policy.AskTimeout,
	}, recipes.FormSchema(missing))

	opened, err := s.reg.broker.OpenRequest(req)
	if err != nil {
		return nil, "", err
	}
	s.reg.publish(events.Event{Kind: events.ElicitOpened, Data: mustJSON(elicitEventSummary(opened))})

	actx, cancel := context.WithTimeout(ctx, s.consumer.policy.AskTimeout)
	defer cancel()
	ans, aerr := s.reg.broker.Await(actx, opened.ID)
	if aerr != nil || ans.Action != elicit.Accept {
		return nil, opened.ID, fmt.Errorf(
			"%s needs %s, and nobody answered within %s (elicitation %s). "+
				"Supply the values with the request instead",
			r.Name, names(missing), s.consumer.policy.AskTimeout, opened.ID)
	}
	var out map[string]any
	if err := json.Unmarshal(ans.Content, &out); err != nil {
		return nil, opened.ID, fmt.Errorf("the answer to %s was not an object: %w", opened.ID, err)
	}
	return out, opened.ID, nil
}

func names(ps []recipes.Placeholder) string {
	var out []string
	for _, p := range ps {
		out = append(out, p.Name)
	}
	return strings.Join(out, ", ")
}

// ---- running a script from the daemon ----

// ExecResult is what running a script produced.
type ExecResult struct {
	ExitCode int    `json:"exitCode"`
	Stdout   string `json:"stdout,omitempty"`
	Stderr   string `json:"stderr,omitempty"`
	Error    string `json:"error,omitempty"`
}

// execScript runs generated or rendered source against this daemon.
//
// This is the seam, not the design. Running scripts belongs in an exec
// service with its own options and its own permission model, and one is being
// built; when it lands, this function becomes a call into it. Until then a
// recipe that cannot be run is a recipe that is half a feature, so it runs
// here, through the same runner the CLI uses, against this daemon's own
// endpoint.
func (s *Server) execScript(ctx context.Context, source, session string) (*ExecResult, error) {
	s.warmIfCold(ctx)
	nss, err := s.reg.CodegenNamespaces(nil, config.Profile{All: true})
	if err != nil {
		return nil, err
	}
	var out, errOut strings.Builder
	timeout := s.policyFor(ctx).RunTimeout
	rctx, cancel := context.WithTimeout(ctx, timeout)
	defer cancel()

	res, rerr := runner.Run(rctx, runner.Options{
		Source:        source,
		Prelude:       prelude(nss),
		ClientSource:  codegen.Module(nss, s.endpoint, session),
		GlobalsSource: codegen.GlobalDeclarations(nss),
		Timeout:       timeout,
		Stdout:        &out,
		Stderr:        &errOut,
		Env: map[string]string{
			"MCPX_SESSION":    session,
			"MCPX_SESSION_ID": session,
			"MCPX_ENDPOINT":   s.endpoint,
		},
	})
	result := &ExecResult{Stdout: out.String(), Stderr: errOut.String()}
	if rerr != nil {
		result.Error = rerr.Error()
		return result, nil
	}
	result.ExitCode = res.ExitCode
	return result, nil
}

// prelude gives a generated snippet the same surface a file script has:
// every namespace as a bare identifier, globals installed.
func prelude(nss []codegen.Namespace) string {
	var names []string
	for _, ns := range nss {
		names = append(names, ns.Name)
	}
	sort.Strings(names)
	var b strings.Builder
	b.WriteString("// --- mcpx prelude (generated) ---\n")
	fmt.Fprintf(&b, "import tools, { call, readResource, log, emit, ToolError, installGlobals } from %q;\n",
		"./"+runner.ClientFileName)
	b.WriteString("installGlobals();\n")
	if len(names) > 0 {
		fmt.Fprintf(&b, "const { %s } = tools;\n", strings.Join(names, ", "))
		b.WriteString("void [" + strings.Join(names, ", ") + "];\n")
	}
	b.WriteString("void [tools, call, readResource, log, emit, ToolError];\n")
	b.WriteString("// --- end prelude ---\n\n")
	return b.String()
}

// ---- generation ----

// generate asks a model provider to write a script.
//
// mcpx has no model, so generation goes to a provider: the daemon's own
// repair.* configuration, or the provider/model the caller supplied on
// this intent request. It no longer falls back to an MCP sampling
// request: sampling is deprecated in MCP 2026-07-28, and the primary
// consumer cannot answer it (#1961). With no provider, generate returns
// errNoModel and the intent route answers with its no-model resolution.
//
// What the provider is given is the part that needs care. The reason mcpx
// is worth having is that tool schemas stay out of a model's context, so
// a generation request sends the slice of the catalog the existing
// search ranked for this request, inside a budget, and nothing else.

func (s *Server) providerConfig() scripting.ProviderConfig {
	if s.set == nil {
		return scripting.ProviderConfig{}
	}
	getString := func(key string) string {
		if v, ok := s.set.Value(key); ok {
			return v.Raw
		}
		return ""
	}
	return scripting.ProviderConfig{
		Type:    getString("repair.provider"),
		BaseURL: getString("repair.url"),
		APIKey:  getString("repair.apiKey"),
		Model:   getString("repair.model"),
		Command: getString("repair.command"),
	}
}

func (s *Server) resolveProvider() scripting.Provider {
	return scripting.ResolveProvider(s.providerConfig(), nil)
}

// callerProviderConfig layers a caller-supplied provider and model (the
// intent request's provider/model fields, from `mcpx prompt --provider
// --model` or the /v1 caller) over the daemon's own configuration. An
// empty caller value leaves the daemon's setting in place.
func callerProviderConfig(base scripting.ProviderConfig, provider, model string) scripting.ProviderConfig {
	if provider != "" {
		base.Type = provider
	}
	if model != "" {
		base.Model = model
	}
	return base
}

// resolveProviderFor resolves the provider for one intent request: the
// caller's provider/model when supplied, the daemon's repair.* provider
// otherwise.
func (s *Server) resolveProviderFor(provider, model string) scripting.Provider {
	return scripting.ResolveProvider(
		callerProviderConfig(s.providerConfig(), provider, model), nil)
}

// model is the caller's EXPLICIT model only; the daemon's own defaults
// are applied per-path below (model.default for the model layer,
// repair.model for the legacy provider's label), never by the caller
// pre-filling one path's default into the other (Codex review, PR #2592).
// session is the intent session, propagated to sampling questions so an
// answerer polling /v1/elicit?session=<session> can see them.
// explicitProvider is the provider the caller named on THIS request
// (empty for none): the caller-over-daemon contract Client.Intent
// documents -- an explicit choice is the generation provider, and the
// daemon's model layer applies only when no override was supplied.
// Letting the layer take precedence sent a request's Claude model id
// to the configured OpenAI endpoint (Codex review, PR #2592).
func (s *Server) generate(ctx context.Context, prompt string, prov scripting.Provider, model string, session string, explicitProvider string) (string, string, error) {
	nss, err := s.reg.CodegenNamespaces(nil, config.Profile{All: true})
	if err != nil {
		return "", "", err
	}
	slice := codegen.Catalog(nss, codegen.CatalogOptions{
		Budget: s.policyFor(ctx).PromptCatalogBudget,
		Bias:   recipes.Terms(prompt),
	})
	system := "You write short TypeScript programs for mcpx. " +
		"Every tool below is already bound as an async function on the named " +
		"namespace; call it directly, import nothing, and end by logging or " +
		"returning the result. Answer with the program only, no prose and no " +
		"code fences.\n\n" + slice

	// The model layer (#1925) is the path when one is configured: the
	// caller resolved model.* through the settings registry, and a
	// provider error here is errNoModel like any other, so the intent
	// route falls back to its deterministic answer -- the ranked recipes
	// plus the note that no model answered -- which is the scope guard's
	// required shape. With model.provider at its default (none) this
	// whole branch is skipped and generation behaves as it always has.
	mp, merr := s.modelLayerWanted(ctx, session, explicitProvider == "")
	if merr != nil {
		// Finding (Codex, PR #2592): a model layer that was configured
		// but cannot resolve (model.provider=plugin, sampling with no
		// path) must NOT fall through to the legacy repair provider --
		// that would silently spend a different provider (an auto-
		// discovered CLI, an env key) the user did not select, possibly
		// against prompt.sample=never. Resolution failure is the
		// deterministic no-model outcome, exactly like a provider error.
		if s.logger != nil {
			s.logger.Printf("model layer: %v; generation falls back to the deterministic answer", merr)
		}
		return "", "", errNoModel
	}
	if mp != nil {
		return s.generateViaModel(ctx, mp, system, prompt, model)
	}

	if prov == nil {
		return "", "", errNoModel
	}
	text, perr := prov.Complete(ctx, scripting.CompletionRequest{
		SystemPrompt: system,
		UserPrompt:   prompt,
		MaxTokens:    s.policyFor(ctx).PromptMaxTokens,
		Temperature:  0.2,
	})
	if perr != nil || strings.TrimSpace(text) == "" {
		return "", "", errNoModel
	}
	used := "provider:" + prov.Name()
	if model == "" {
		// The legacy path's own default: the daemon's repair.model.
		// (The model layer never sees this value; see generateViaModel.)
		model = s.providerConfig().Model
	}
	if model != "" {
		used += "/" + model
	}
	return stripFences(text), used, nil
}

// errNoModel means no provider answered: mcpx has no model of its own,
// and generation no longer falls back to an MCP sampling request
// (sampling is deprecated in MCP 2026-07-28; #1961).
var errNoModel = fmt.Errorf("no model provider answered")

// modelLayer resolves the #1925 provider layer for this request from the
// request's own policy. (nil, nil) means no layer applies -- model.provider
// none or unset, or auto with nothing configured -- and the caller keeps
// the behaviour it had before the layer existed. A non-nil error means a
// provider WAS configured but cannot be built (plugin, sampling with no
// path): the caller must surface that as no-model, never silently invoke
// a different provider in its place (Codex review, PR #2592).
// modelLayerWanted is modelLayer behind the caller's explicit-provider
// choice: an intent request that named its own provider does not consult
// the daemon's model layer at all.
func (s *Server) modelLayerWanted(ctx context.Context, session string, wanted bool) (model.Provider, error) {
	if !wanted {
		return nil, nil
	}
	return s.modelLayer(ctx, session)
}

func (s *Server) modelLayer(ctx context.Context, session string) (model.Provider, error) {
	pol := s.policyFor(ctx)
	if pol.ModelProvider == "" || pol.ModelProvider == "none" {
		return nil, nil
	}
	return model.Resolve(model.Config{
		Provider:     pol.ModelProvider,
		BaseURL:      pol.ModelBaseURL,
		APIKeyEnv:    pol.ModelAPIKeyEnv,
		DefaultModel: pol.ModelDefault,
		Timeout:      pol.ModelTimeout,
		Sampler:      s.brokerSampler(session),
	})
}

// generateViaModel is generate's model-layer path: the same system prompt
// and catalog slice, one Generate call, the layer's bounds applied --
// model.budget caps the tokens asked for, model.default supplies the
// model id when the caller named none.
func (s *Server) generateViaModel(ctx context.Context, prov model.Provider, system, prompt, modelID string) (string, string, error) {
	pol := s.policyFor(ctx)
	if modelID == "" {
		modelID = pol.ModelDefault
	}
	maxTokens := modelMaxTokens(pol.PromptMaxTokens, pol.ModelBudget)
	resp, err := prov.Generate(ctx, model.Request{
		Model:     modelID,
		Messages:  []model.Message{{Role: "system", Content: system}, {Role: "user", Content: prompt}},
		MaxTokens: maxTokens,
	})
	if err != nil || strings.TrimSpace(resp.Text) == "" {
		return "", "", errNoModel
	}
	used := "provider:" + prov.Name()
	answered := resp.Model
	if answered == "" {
		answered = model.ModelID(modelID)
	}
	if answered != "" {
		used += "/" + answered
	}
	return stripFences(resp.Text), used, nil
}

// modelMaxTokens applies the model layer's token bounds to one call:
// the per-call ask (prompt.maxTokens), capped by model.budget when one
// is configured. A nonpositive per-call value is UNSET, never a value:
// the OpenAI backend omits a nonpositive limit, which would leave the
// endpoint's own limit in force and silently bypass a configured
// budget ceiling (Codex review, PR #2592).
func modelMaxTokens(perCall, budget int) int {
	max := perCall
	if max < 0 {
		max = 0
	}
	if budget > 0 && (max == 0 || budget < max) {
		max = budget
	}
	return max
}

// modelRepairProvider adapts the resolved model-layer backend to the
// scripting.Provider shape Repair speaks, so a script the model layer
// generated is repaired by the SAME backend -- under the layer's budget
// -- and never by an independently resolved legacy provider the user
// did not select for this generation: with prompt.sample=never and a
// local model.baseURL, the old code sent the generated script and the
// catalog to an auto-discovered provider on validation failure (Codex
// review, PR #2592).
type modelRepairProvider struct {
	s       *Server
	mp      model.Provider
	modelID string
}

func (p modelRepairProvider) Name() string { return p.mp.Name() }

func (p modelRepairProvider) Complete(ctx context.Context, req scripting.CompletionRequest) (string, error) {
	pol := p.s.policyFor(ctx)
	modelID := p.modelID
	if modelID == "" {
		modelID = pol.ModelDefault
	}
	resp, err := p.mp.Generate(ctx, model.Request{
		Model: modelID,
		Messages: []model.Message{
			{Role: "system", Content: req.SystemPrompt},
			{Role: "user", Content: req.UserPrompt},
		},
		MaxTokens: modelMaxTokens(req.MaxTokens, pol.ModelBudget),
	})
	if err != nil {
		return "", err
	}
	return resp.Text, nil
}

// brokerSampler is the sampling backend's path into the elicitation
// broker: a generation request becomes a sampling question for whatever
// drives mcpx, in the CreateMessage shape answerers already know
// (elicit.SampleAsk). Nobody answering within the deadline is an
// unavailable provider, not a hang: the broker's expiry answer lands as
// an error here and the caller falls back deterministically.
func (s *Server) brokerSampler(session string) model.SampleFunc {
	if s.reg == nil || s.reg.broker == nil {
		return nil
	}
	broker := s.reg.broker
	return func(ctx context.Context, req model.Request) (model.Response, error) {
		params, err := samplingParams(req)
		if err != nil {
			return model.Response{}, err
		}
		ttl := s.policyFor(ctx).ModelTimeout
		if ttl <= 0 {
			ttl = defaults.PromptSampleTimeout
		}
		// Session is the intent session, so Broker.Pending's exact
		// session filter can show this question to the answerer polling
		// /v1/elicit?session=<session> for it (Codex, PR #2592).
		question, err := broker.OpenRequest(elicit.SampleAsk(elicit.Ask{
			Server:  "mcpx",
			Session: session,
			Message: "mcpx asks for a model completion (model.provider=sampling)",
			TTL:     ttl,
		}, params))
		if err != nil {
			return model.Response{}, err
		}
		// model.timeout must bound the WAIT, not only the question's
		// initial TTL: an answerer reading the question via
		// /v1/elicit/{id} renews its expiry (Broker.Get touches), so a
		// TTL alone can be extended toward elicit.maxAge by repeated
		// reads. Await honours ctx, so the deadline goes on the ctx
		// (Codex review, PR #2592).
		awaitCtx, cancel := context.WithTimeout(ctx, ttl)
		defer cancel()
		ans, err := broker.Await(awaitCtx, question.ID)
		if err != nil {
			return model.Response{}, err
		}
		if ans.Action != elicit.Accept {
			return model.Response{}, fmt.Errorf("model: sampling %s: %w", ans.Action, model.ErrUnavailable)
		}
		text, answered := parseSampleResult(ans.Content)
		if strings.TrimSpace(text) == "" {
			return model.Response{}, fmt.Errorf("model: sampling answered with no text: %w", model.ErrUnavailable)
		}
		return model.Response{Text: text, Model: answered, Provider: "sampling"}, nil
	}
}

// samplingParams renders a model.Request as an MCP sampling
// CreateMessageRequest, the shape the broker's answerers speak.
func samplingParams(req model.Request) (json.RawMessage, error) {
	var messages []map[string]any
	var system string
	for _, m := range req.Messages {
		if m.Role == "system" {
			system = m.Content
			continue
		}
		role := m.Role
		if role == "" {
			role = "user"
		}
		messages = append(messages, map[string]any{
			"role":    role,
			"content": map[string]any{"type": "text", "text": m.Content},
		})
	}
	doc := map[string]any{"messages": messages}
	if system != "" {
		doc["systemPrompt"] = system
	}
	if req.MaxTokens > 0 {
		doc["maxTokens"] = req.MaxTokens
	}
	if len(req.Stop) > 0 {
		doc["stopSequences"] = req.Stop
	}
	if id := model.ModelID(req.Model); id != "" {
		doc["modelPreferences"] = map[string]any{
			"hints": []map[string]any{{"name": id}},
		}
	}
	return json.Marshal(doc)
}

// parseSampleResult extracts the text and the answering model from a
// sampling CreateMessageResult. Content arrives as one block (the legacy
// shape) or an array of blocks (2025-11-25); text blocks are concatenated.
func parseSampleResult(raw json.RawMessage) (text, modelName string) {
	var result struct {
		Model   string          `json:"model"`
		Content json.RawMessage `json:"content"`
	}
	if json.Unmarshal(raw, &result) != nil {
		return "", ""
	}
	var blocks []struct {
		Type string `json:"type"`
		Text string `json:"text"`
	}
	if json.Unmarshal(result.Content, &blocks) == nil {
		var parts []string
		for _, b := range blocks {
			if b.Type == "text" {
				parts = append(parts, b.Text)
			}
		}
		return strings.Join(parts, ""), result.Model
	}
	var one struct {
		Type string `json:"type"`
		Text string `json:"text"`
	}
	if json.Unmarshal(result.Content, &one) == nil && one.Type == "text" {
		return one.Text, result.Model
	}
	return "", result.Model
}

// stripFences removes a markdown code fence a model wrapped the answer in.
//
// Asked for "the program only" it usually obliges; when it does not, the
// alternative to this is a script whose first line is three backticks and a
// syntax error nobody can explain.
func stripFences(s string) string {
	t := strings.TrimSpace(s)
	if !strings.HasPrefix(t, "```") {
		return t
	}
	if i := strings.IndexByte(t, '\n'); i >= 0 {
		t = t[i+1:]
	}
	if j := strings.LastIndex(t, "```"); j >= 0 {
		t = t[:j]
	}
	return strings.TrimSpace(t)
}

type policyKey struct{}

// withPolicy carries a request's own consumer policy -- its call-scoped
// settings, resolved from the daemon's and the caller's layers -- down to
// the functions that run and generate for it.
//
// The daemon used to read these once, from its config files, at start: a
// caller's recipes.limit, prompt.runTimeout or prompt.autonomy from the
// environment, a flag or PUT /v1/settings never reached them, while the PUT
// answered "applied".
func withPolicy(ctx context.Context, p ConsumerPolicy) context.Context {
	return context.WithValue(ctx, policyKey{}, p)
}

// policyFor is the policy of the request ctx belongs to, or the daemon's.
func (s *Server) policyFor(ctx context.Context) ConsumerPolicy {
	if p, ok := ctx.Value(policyKey{}).(ConsumerPolicy); ok {
		return p
	}
	return s.consumer.policy
}

// warmIfCold reads the schemas when no server has any yet.
//
// A daemon that has just started reads them in the background. A recipe run,
// a generated script or a diagnosis that arrived first used the empty
// catalog: the client was generated with no namespaces, so the script died
// on "demo.echo is not a function", and a diagnosis compared the source with
// nothing and found nothing wrong. The CLI's own commands wait for the
// schemas; these routes are also reached by the plugin and by MCP, which do
// not. "No server has any" rather than "some server has none", as the CLI's
// ensureAnySchemas does it, so one server that cannot start does not put its
// start timeout on every request.
func (s *Server) warmIfCold(ctx context.Context) {
	s.reg.mu.RLock()
	cold := len(s.reg.pools) > 0
	for _, p := range s.reg.pools {
		if _, _, at := p.CachedSchemas(); !at.IsZero() {
			cold = false
			break
		}
	}
	s.reg.mu.RUnlock()
	if cold {
		s.reg.Warm(ctx, false)
	}
}
