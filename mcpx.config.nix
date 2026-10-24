# SPDX-License-Identifier: MIT OR Apache-2.0
#
# mcpx's configuration (~/.config/mcpx/config.json), derived from the repo's
# native .mcpx.json configuration.
{ lib }:
let
  cfg = builtins.fromJSON (builtins.readFile ./.mcpx.json);

  pooling = {
    github.excludeTools = [
      "create_repository"
      "fork_repository"
      "delete_repository"
    ];
  };
in
{
  mcpServers = lib.mapAttrs (
    name: server: server // lib.optionalAttrs (pooling ? ${name}) { mcpx = pooling.${name}; }
  ) cfg.mcpServers;
}
