{
  lib,
  stdenvNoCC,
  fetchurl,
  autoPatchelfHook,
  installShellFiles,
  makeWrapper,
  zlib,
  stdenv,
}:

# Upstream prebuilt release binaries for tools where nixpkgs trails upstream.
#
# Why prebuilt rather than overriding `src` on the nixpkgs derivation:
#   Bumping src on a Go/Rust package also invalidates vendorHash/cargoHash, and
#   every one of these would then compile from source with zero cache hits --
#   opentelemetry-collector-contrib alone is a very large Go build. Upstream
#   publishes signed, statically linked release artifacts for all of these, so
#   fetching those is both faster and closer to what upstream actually tests.
#
# Tradeoffs, stated plainly:
#   * These are x86_64-linux only. Each entry asserts on that rather than
#     silently producing a broken package elsewhere.
#   * Being prebuilt, they are opaque: no source bootstrap, no patches, and
#     nixpkgs security backports do not apply. That is the deal you take for
#     being current.
#   * Versions here are pinned by hash and will NOT move with the nixpkgs
#     channel. They go stale silently. Re-run the survey periodically.
#
# NOT included, deliberately:
#   * skopeo  -- upstream publishes zero release assets (verified: 0 assets on
#                v1.24.1), so there is no binary to fetch. nixpkgs 1.24.0 vs
#                1.24.1 is a patch, not worth a source build.
#   * cosign, dive, honeymarker, otel-cli -- nixpkgs is already AT the latest
#                upstream release. dive/honeymarker/otel-cli have commits since
#                their last tag, but those upstreams have not cut a release in
#                1-2 years; tracking their main would mean shipping unreleased
#                code from dormant projects.
#   * oci-cli  -- distributed via PyPI, not GitHub release binaries; handled
#                separately.
#
# To update: re-run the survey, then for each entry
#   nix store prefetch-file --json --name <asset> <url>

let
  inherit (stdenv.hostPlatform) system;

  # Common shape: fetch one release asset, drop binaries into $out/bin.
  #   binaries   -- paths inside the unpacked tree to install
  #   stripRoot  -- archive has a single top-level directory
  #   bare       -- asset is the executable itself, not an archive
  mk =
    {
      pname,
      version,
      url,
      hash,
      binaries ? [ pname ],
      bare ? false,
      stripComponents ? null,
      extraInstall ? "",
      description,
      homepage,
      license,
      mainProgram ? pname,
      dynamic ? false,
    }:
    assert lib.assertMsg (system == "x86_64-linux")
      "pkgs/latest-cli: ${pname} is a prebuilt x86_64-linux artifact; system is ${system}";
    stdenvNoCC.mkDerivation {
      inherit pname version;

      src = fetchurl { inherit url hash; };

      # Bare executables are not archives; nothing to unpack.
      dontUnpack = bare;

      nativeBuildInputs = [ installShellFiles ] ++ lib.optional dynamic autoPatchelfHook;
      # libgcc_s.so.1 comes from the compiler runtime, not glibc; tursodb needs it.
      buildInputs = lib.optionals dynamic [
        zlib
        stdenv.cc.cc.lib
      ];

      # Upstream ships these already stripped and, for the Go/Rust ones,
      # statically linked. Re-stripping gains nothing and can break buildIds.
      dontStrip = true;

      unpackPhase = lib.optionalString (!bare) ''
        runHook preUnpack
        mkdir -p source && cd source
        tar --extract --file="$src" \
          ${lib.optionalString (stripComponents != null) "--strip-components=${toString stripComponents}"}
        runHook postUnpack
      '';

      installPhase = ''
        runHook preInstall
        mkdir -p "$out/bin"
        ${
          if bare then
            ''install -Dm755 "$src" "$out/bin/${mainProgram}"''
          else
            lib.concatMapStringsSep "\n" (b: ''
              install -Dm755 "${b}" "$out/bin/$(basename "${b}")"
            '') binaries
        }
        ${extraInstall}
        runHook postInstall
      '';

      doInstallCheck = true;
      installCheckPhase = ''
        runHook preInstallCheck
        # Several of these (turso-cli at least) read or create config under
        # $HOME and exit non-zero when it is unset or unwritable, which is the
        # case in the build sandbox.
        export HOME="$TMPDIR"
        "$out/bin/${mainProgram}" --version > /dev/null 2>&1 \
          || "$out/bin/${mainProgram}" version > /dev/null 2>&1 \
          || { echo "${pname}: neither --version nor version worked" >&2; exit 1; }
        runHook postInstallCheck
      '';

      meta = {
        inherit description homepage license mainProgram;
        platforms = [ "x86_64-linux" ];
        sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
      };
    };
in
{
  flyctl = mk {
    pname = "flyctl";
    version = "0.4.111";
    url = "https://github.com/superfly/flyctl/releases/download/v0.4.111/flyctl_0.4.111_Linux_x86_64.tar.gz";
    hash = "sha256-GHjX+x+KQYA5BCzwdJ5LchbGxwGLZq4QYRpkwrbZqp4=";
    description = "Command-line tool for fly.io";
    homepage = "https://fly.io/";
    license = lib.licenses.asl20;
  };

  gh = mk {
    pname = "gh";
    version = "2.102.0";
    url = "https://github.com/cli/cli/releases/download/v2.102.0/gh_2.102.0_linux_amd64.tar.gz";
    hash = "sha256-u3ZvcQ7vjt6FnBhXjHLDJ1l81MioWwYAGx84Q8YBk4Y=";
    stripComponents = 1;
    binaries = [ "bin/gh" ];
    extraInstall = ''
      if [ -d share/man/man1 ]; then
        mkdir -p "$out/share/man/man1"
        cp share/man/man1/*.1 "$out/share/man/man1/"
      fi
      installShellCompletion --cmd gh \
        --bash <("$out/bin/gh" completion -s bash) \
        --zsh  <("$out/bin/gh" completion -s zsh) \
        --fish <("$out/bin/gh" completion -s fish)
    '';
    description = "GitHub CLI";
    homepage = "https://cli.github.com/";
    license = lib.licenses.mit;
  };

  codex = mk {
    pname = "codex";
    version = "0.160.0";
    url = "https://github.com/openai/codex/releases/download/rust-v0.160.0/codex-x86_64-unknown-linux-musl.tar.gz";
    hash = "sha256-MGhlQX1O56kneFhSkQpSf0Hh4Vmt05CsWuOsy2fUShM=";
    binaries = [ "codex-x86_64-unknown-linux-musl" ];
    # Archive contains the musl-static binary under its target triple name.
    extraInstall = ''
      mv "$out/bin/codex-x86_64-unknown-linux-musl" "$out/bin/codex"
    '';
    description = "OpenAI Codex coding agent CLI";
    homepage = "https://github.com/openai/codex";
    license = lib.licenses.asl20;
  };

  ovhcloud-cli = mk {
    pname = "ovhcloud-cli";
    version = "0.15.0";
    url = "https://github.com/ovh/ovhcloud-cli/releases/download/v0.15.0/ovhcloud-cli_Linux_x86_64.tar.gz";
    hash = "sha256-EQnRV9z2d786dhbksntP47XMmE4cyCmHWSAWVBpsiO0=";
    binaries = [ "ovhcloud" ];
    mainProgram = "ovhcloud";
    description = "OVHcloud command-line interface";
    homepage = "https://github.com/ovh/ovhcloud-cli";
    license = lib.licenses.asl20;
  };

  cloudflared = mk {
    pname = "cloudflared";
    version = "2026.9.3";
    url = "https://github.com/cloudflare/cloudflared/releases/download/2026.9.3/cloudflared-linux-amd64";
    hash = "sha256-d+JtjZAOC4Rp9BYjnRS18pZSX995/ub1Ee9VYJ4/usI=";
    bare = true;
    description = "Cloudflare Tunnel client";
    homepage = "https://github.com/cloudflare/cloudflared";
    license = lib.licenses.asl20;
  };

  oras = mk {
    pname = "oras";
    version = "1.3.4";
    url = "https://github.com/oras-project/oras/releases/download/v1.3.4/oras_1.3.4_linux_amd64.tar.gz";
    hash = "sha256-8nrbk1Ai2U343HdxnDIt2lkseKDVem99zdjZALJIxFQ=";
    description = "OCI registry client for arbitrary artifacts";
    homepage = "https://oras.land/";
    license = lib.licenses.asl20;
  };

  regctl = mk {
    pname = "regctl";
    version = "0.11.6";
    url = "https://github.com/regclient/regclient/releases/download/v0.11.6/regctl-linux-amd64";
    hash = "sha256-jg5ipJf824BI0YqpJ6E5YTF2ugUx9BK8VBBE4o+YVr0=";
    bare = true;
    description = "Registry client for OCI/Docker registries";
    homepage = "https://github.com/regclient/regclient";
    license = lib.licenses.asl20;
  };

  opentelemetry-collector-contrib = mk {
    pname = "opentelemetry-collector-contrib";
    version = "0.162.0";
    url = "https://github.com/open-telemetry/opentelemetry-collector-releases/releases/download/v0.162.0/otelcol-contrib_0.162.0_linux_amd64.tar.gz";
    hash = "sha256-/MBjdJ9zD4wh/iny00D/F09fHFiFvTFW+2yYWjA2/MM=";
    binaries = [ "otelcol-contrib" ];
    mainProgram = "otelcol-contrib";
    description = "OpenTelemetry Collector, contrib distribution";
    homepage = "https://github.com/open-telemetry/opentelemetry-collector-releases";
    license = lib.licenses.asl20;
  };

  go-containerregistry = mk {
    pname = "go-containerregistry";
    version = "0.22.1";
    url = "https://github.com/google/go-containerregistry/releases/download/v0.22.1/go-containerregistry_Linux_x86_64.tar.gz";
    hash = "sha256-Creh1pMqITrtlkzpdmbDB3/mkchgZBNnSos+C57EzaA=";
    binaries = [
      "crane"
      "gcrane"
      "krane"
    ];
    mainProgram = "crane";
    description = "Tools for interacting with remote container images (crane)";
    homepage = "https://github.com/google/go-containerregistry";
    license = lib.licenses.asl20;
  };

  turso-cli = mk {
    pname = "turso-cli";
    version = "1.0.33";
    url = "https://github.com/tursodatabase/turso-cli/releases/download/v1.0.33/turso-cli_Linux_x86_64.tar.gz";
    hash = "sha256-81ldSvM4XmkfaBUq4ksf+jdoQT+gB8PMz7NoGTf1JQI=";
    binaries = [ "turso" ];
    mainProgram = "turso";
    # Unlike the Go tools here, this one is dynamically linked against glibc
    # (interpreter /lib64/ld-linux-x86-64.so.2, needs libc/libdl/libpthread),
    # so it will not run on NixOS without autoPatchelfHook.
    dynamic = true;
    extraInstall = ''
      installShellCompletion --cmd turso \
        --bash completions/turso.bash \
        --zsh  completions/turso.zsh \
        --fish completions/turso.fish
    '';
    description = "CLI for Turso";
    homepage = "https://turso.tech";
    license = lib.licenses.mit;
  };

  turso = mk {
    pname = "turso";
    version = "0.8.1";
    url = "https://github.com/tursodatabase/turso/releases/download/v0.8.1/turso_cli-x86_64-unknown-linux-gnu.tar.xz";
    hash = "sha256-tLlPM0zIzL9qfN4ap8KUmsvFHIPcQWgBkqR71hnAIes=";
    stripComponents = 1;
    binaries = [ "tursodb" ];
    mainProgram = "tursodb";
    dynamic = true;
    description = "Interactive SQL shell for Turso";
    homepage = "https://github.com/tursodatabase/turso";
    license = lib.licenses.mit;
  };

  honeycomb-refinery = mk {
    pname = "honeycomb-refinery";
    version = "3.4.0";
    url = "https://github.com/honeycombio/refinery/releases/download/v3.4.0/refinery-linux-amd64";
    hash = "sha256-sRiPlz/sNUYvdd2LOkGZH2oB0TlAwxnd2Vd5s4mHqj0=";
    bare = true;
    mainProgram = "refinery";
    description = "Honeycomb trace-aware tail sampling proxy";
    homepage = "https://github.com/honeycombio/refinery";
    license = lib.licenses.asl20;
  };

  backblaze-b2 = mk {
    pname = "backblaze-b2";
    version = "5.0.0";
    url = "https://github.com/Backblaze/B2_Command_Line_Tool/releases/download/v5.0.0/b2-linux";
    hash = "sha256-FIIwXN3N0/H7Gs2+esCM6how+EfdHpk92vEZAGwwrxM=";
    bare = true;
    mainProgram = "b2";
    description = "Backblaze B2 command-line tool";
    homepage = "https://github.com/Backblaze/B2_Command_Line_Tool";
    license = lib.licenses.mit;
  };

  # t3code's server distribution. Not built with `mk`: this is a whole tree
  # (binary + bundled node_modules + the `client/` web UI, which is a PWA) that
  # has to stay together, not a single binary to drop into bin/.
  #
  # Upstream is pingdotgg/t3code. The npm `t3` package is only a launcher that
  # pulls a platform-specific optional dependency, and it prints "The desktop
  # app and release archives are at .../releases" -- so the release tarball is
  # the real artifact.
  #
  # This provides the `t3` CLI/server (used by systemd.user.services.t3code).
  # The Electron desktop GUI is a separate asset (.deb/.AppImage) and is still
  # taken from nixpkgs as `t3code-desktop`.
  t3 =
    assert lib.assertMsg (system == "x86_64-linux")
      "pkgs/latest-cli: t3 is a prebuilt x86_64-linux artifact; system is ${system}";
    stdenvNoCC.mkDerivation {
      pname = "t3";
      version = "0.0.45";

      src = fetchurl {
        url = "https://github.com/pingdotgg/t3code/releases/download/v0.0.45/t3-0.0.45-linux-x64.tar.gz";
        hash = "sha256-EFBa50vGpDz6sP3gvwag4NaGL3QDBZG+mUpkDUig1r0=";
      };

      nativeBuildInputs = [
        autoPatchelfHook
        makeWrapper
      ];
      buildInputs = [
        zlib
        stdenv.cc.cc.lib
      ];

      sourceRoot = "t3-0.0.45-linux-x64";
      dontStrip = true;

      installPhase = ''
        runHook preInstall
        mkdir -p "$out/libexec/t3" "$out/bin"
        cp -r . "$out/libexec/t3/"
        chmod +x "$out/libexec/t3/t3"
        # The binary resolves client/ and node_modules/ relative to itself, so
        # wrap rather than symlink into bin/.
        makeWrapper "$out/libexec/t3/t3" "$out/bin/t3"
        runHook postInstall
      '';

      doInstallCheck = true;
      installCheckPhase = ''
        runHook preInstallCheck
        export HOME="$TMPDIR"
        "$out/bin/t3" --version > /dev/null
        runHook postInstallCheck
      '';

      meta = {
        description = "T3 Code server and CLI";
        homepage = "https://t3.codes";
        license = lib.licenses.unfree;
        mainProgram = "t3";
        platforms = [ "x86_64-linux" ];
        sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
      };
    };
}
