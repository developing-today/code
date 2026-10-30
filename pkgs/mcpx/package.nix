{
  lib,
  # Built with the highest released Go toolchain (1.27.x); go.mod's `go`
  # directive tracks it, so buildGo127Module rather than plain buildGoModule.
  buildGo127Module,
  # Runtimes mcpx can use for `mcpx run` / `mcpx exec`. It picks the first one
  # present on PATH in this order, so the wrapper below pins a known-good set
  # rather than depending on whatever the calling shell happens to have.
  deno,
  bun,
  bun-bin ? bun,
  nodejs,
  # git is the fallback for the `repo` and `worktree` scopes: mcpx reads .git
  # itself and only asks git about layouts it does not vouch for
  # (docs/git-discovery.md). Kept on the wrapper's PATH so those still
  # resolve; the container image leaves it out.
  git,
  makeBinaryWrapper,
  installShellFiles,
}:
let
  # The mcpx version's single source is the VERSION file beside this
  # one (decision 0005 §3, #1881; #1895's one-literal rule, moved):
  # the derivation attribute below and the ldflags build stamp both
  # take this binding, because the argument to buildGo127Module is a
  # non-recursive attribute set and its own attributes are not in
  # scope for interpolation inside it. internal/buildcheck guards
  # the scheme.
  version = lib.removeSuffix "\n" (builtins.readFile ./VERSION);
in
buildGo127Module {
  pname = "mcpx";
  inherit version;

  src = lib.cleanSourceWith {
    src = ./.;
    filter =
      path: type:
      let
        base = baseNameOf path;
      in
      !(base == "bin" || base == "result" || lib.hasPrefix "result-" base || base == "mcpx-client.ts");
  };

  # Five direct Go dependencies.
  #
  # modernc.org/sqlite backs the log index. It is a pure-Go translation rather
  # than the usual mattn driver, because a cgo driver would make this
  # derivation need a C toolchain and break cross-compilation, for a database
  # that is only ever a rebuildable index over the JSONL logs.
  #
  # bubbletea, bubbles and lipgloss are the full-screen browser. They cost
  # 1.2 MB in the stripped binary and replace what would otherwise be raw
  # terminal handling: escape sequences, resize, and a redraw loop. That is
  # the kind of code that is never finished and never correct on every
  # terminal.
  #
  # gopkg.in/yaml.v3 reads OpenAPI documents, which are published as YAML as
  # often as JSON and which one you get is not the caller's choice.
  #
  # Everything else -- the MCP client, the process pools, the JSON Schema to
  # TypeScript compiler, the CLI -- is standard library.
  vendorHash = "sha256-XTK49aropeKTGu/zlbjgtbT0BlktvUhsOfdpHrKoc5M=";

  subPackages = [ "cmd/mcpx" ];

  # Pure-Go build, matching the design comment above: no cgo, no C toolchain
  # needed, cross-compilation stays intact, and the result is a static binary.
  # docs/container.md measured a CGO_ENABLED=0 build working end-to-end.
  env.CGO_ENABLED = "0";

  # No VCS stamping. git is on PATH during the build because the check phase
  # needs it (nativeCheckInputs is on PATH in every phase), and with git found
  # `go install` stamps VCS info by walking up from the build directory with
  # plain stat. Where the sandbox build directory sits under an unrelated git
  # checkout, it finds that repo, runs `git status` there as the build user,
  # and dies with "error obtaining VCS status: exit status 128" (#304). Where
  # it does not die it would embed the wrong repository's state. The source is
  # a store path, not a checkout, so there is nothing true to stamp.
  preBuild = ''
    export GOFLAGS="$GOFLAGS -buildvcs=false"
  '';

  # The version comes from VERSION (above) and is interpolated here, so
  # the build stamp cannot silently disagree with the package version.
  # The default in cmd/mcpx/main.go is deliberately NOT this version:
  # it is the 0.0.0-dev sentinel (decision 0005 §3), so a binary built
  # without ldflags announces that fact instead of impersonating a
  # release. internal/buildcheck guards both halves of that scheme.
  ldflags = [
    "-s"
    "-w"
    "-X main.version=${version}"
  ];

  nativeBuildInputs = [
    makeBinaryWrapper
    installShellFiles
  ];

  # subPackages confines `go test` to cmd/mcpx, which has none, so the check
  # phase picks its own packages: every package with test files, computed here
  # so a new one is tested by default, minus checkExclude below. Each entry in
  # checkExclude needs a comment on the line above it saying why it cannot run
  # in the sandbox; internal/buildcheck enforces both.
  # The exact `go test -timeout 30m $pkgs` line in the checkPhase below is
  # load-bearing downstream (#418): dezren39/nix's flake.nix rewrites that
  # line to add its darwin takeover/handoff -skip list. That rewrite now
  # fails loudly there (a throw plus the mcpx-darwin-skip flake check and a
  # CI eval step) when the line stops matching, but changing the line here
  # still requires updating the pattern in dezren39/nix's flake.nix in the
  # same change set, or its next mcpx-src pin bump will fail evaluation.
  doCheck = true;
  # The runner and e2e suites spawn real scripts, so they get the same runtimes
  # the wrapper pins.
  nativeCheckInputs = [
    deno
    bun-bin
    nodejs
    git
  ];
  checkPhase =
    let
      checkExclude = [
        # Empty: every tested package, internal/e2e included, passes in the
        # sandbox. Add "internal/<pkg>" here with a comment naming what the
        # sandbox lacks.
      ];
    in
    ''
      runHook preCheck
      export HOME=$TMPDIR/home MCPX_STATE_DIR=$TMPDIR/state MCPX_CACHE_DIR=$TMPDIR/cache
      mkdir -p "$HOME"
      pkgs=$(go list -f '{{if or .TestGoFiles .XTestGoFiles}}{{.ImportPath}}{{end}}' ./... \
        | grep -vxF -e "" ${
          lib.concatMapStringsSep " " (p: "-e github.com/dezren39/mcpx/${p}") checkExclude
        })
      echo "testing $(echo "$pkgs" | wc -l) packages: $pkgs"
      # Go's default per-package timeout is 10m. On a 2-CPU machine the
      # internal/e2e package was still making progress when that cap killed
      # it, which failed the whole mcpx build and blocked the packages that
      # depend on it (dezren39/nix#510). The timeout is per package, so one
      # explicit 30m budget covers e2e without sharding the suite; the
      # other packages finish far under it.
      go test -timeout 30m $pkgs
      runHook postCheck
    '';

  postInstall = ''
    # Generated by the binary rather than checked in, so the page describes the
    # commands and settings this build actually has. A man page maintained
    # separately is wrong within two releases.
    mkdir -p $out/share/man/man1
    $out/bin/mcpx man > $out/share/man/man1/mcpx.1

    installShellCompletion --cmd mcpx \
      --bash <($out/bin/mcpx completion bash) \
      --zsh <($out/bin/mcpx completion zsh) \
      --fish <($out/bin/mcpx completion fish)

    wrapProgram $out/bin/mcpx \
      --suffix PATH : ${
        lib.makeBinPath [
          git
          deno
          bun-bin
          nodejs
        ]
      }
  '';

  meta = {
    description = "Run TypeScript against your MCP servers from the command line";
    longDescription = ''
      mcpx keeps a local daemon that owns every MCP server process. Scripts
      import a generated, fully typed client and call tools as ordinary async
      functions, so only what a script prints reaches the model's context.

      Stateful servers such as chrome-devtools-mcp are leased per session, so
      concurrent agents get separate processes instead of corrupting one
      shared browser.
    '';
    homepage = "https://github.com/dezren39/mcpx";
    license = lib.licenses.mit;
    mainProgram = "mcpx";
    platforms = lib.platforms.unix;
  };
}
