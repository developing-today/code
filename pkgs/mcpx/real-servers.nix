{
  lib,
  buildEnv,
  # The reference MCP servers, from the pinned nixpkgs. Their versions
  # are whatever the flake's nixpkgs pin carries — that pin IS the
  # version set for the real-software subset (#1990), reviewable as a
  # flake.lock diff and reported by `nix flake metadata`.
  mcp-server-time,
  mcp-server-memory,
  mcp-server-filesystem,
  mcp-server-git,
  mcp-server-fetch,
  mcp-server-sequential-thinking,
}:
# mcpx-real-servers: one directory of real third-party MCP servers for
# the realsoftware test subset (pkgs/mcpx/internal/e2e/real_*_test.go).
# Tests resolve servers from this directory via MCPX_REAL_SERVERS_DIR
# (testsupport.RealServer); the nightly workflow builds this derivation
# and points the variable at it. Not in the subset yet: server-everything
# and playwright-mcp — server-everything is not in nixpkgs (it needs its
# own packaging under pkgs/), and playwright-mcp's browser legs belong
# with the tier-2 container work; both are recorded on #1990.
buildEnv {
  name = "mcpx-real-servers";
  # Only bin/ is linked: mcp-server-filesystem and mcp-server-memory are
  # built from the same upstream monorepo and ship an identical
  # lib/node_modules tree, which collides in a full buildEnv merge.
  # Their bin entries resolve into their own store paths, so /bin is
  # all the subset needs (testsupport.RealServer looks under bin/).
  pathsToLink = [ "/bin" ];
  paths = [
    mcp-server-time
    mcp-server-memory
    mcp-server-filesystem
    mcp-server-git
    mcp-server-fetch
    mcp-server-sequential-thinking
  ];
  meta = {
    description = "Flake-pinned real MCP servers for the mcpx realsoftware test subset";
    license = lib.licenses.mit;
  };
}
