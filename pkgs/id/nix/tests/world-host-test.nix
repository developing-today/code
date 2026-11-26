# NixOS VM test for a deployed world host (#10697, P5 of the hosted-worlds
# v1 design): one `id serve --world-host` instance under the NixOS module,
# driven by the operator CLI over its control socket as root.
#
# Usage:
#   pkgs.testers.runNixOSTest (import ./world-host-test.nix { inherit idPackage; })
{ idPackage }:
{
  name = "id-world-host";

  nodes.server =
    { ... }:
    {
      imports = [ ../id-module.nix ];

      # A test token only: a real deployment keeps this file out of the store.
      environment.etc."id-world.env".text = "ID_WORLD_ADMIN_TOKEN=vm-test-admin-token\n";

      services.id = {
        package = idPackage;
        instances.primary = {
          enable = true;
          web = false;
          noRelay = true;
          noGossip = true;
          noMdns = true;
          environmentFile = "/etc/id-world.env";
          world = {
            enable = true;
            host = true;
          };
        };
      };

      environment.systemPackages = [ idPackage ];
    };

  globalTimeout = 600;

  testScript = ''
    import json

    ROOT = "/var/lib/id-primary/.id-worlds"

    def state(world):
        seen = json.loads(server.succeed(f"id host inspect {ROOT} {world} --json"))
        return seen["state"]

    def wait_for_node():
        server.wait_for_unit("id-primary.service")
        server.wait_until_succeeds(f"test -S {ROOT}/host.sock", timeout=120)

    start_all()
    wait_for_node()

    # The default world opens at startup and is recorded live.
    census = json.loads(server.succeed(f"id host list {ROOT} --json"))
    assert [w["name"] for w in census["worlds"]] == ["lobby"], census
    assert state("lobby") == "live"

    # A second host on the same data root is refused while the node answers.
    server.fail(f"timeout 30 id host run {ROOT}")

    # The node exports a live world with its own host key.
    server.succeed(f"id host export {ROOT} lobby /var/lib/id-primary/lobby.idworld")
    server.succeed("test -s /var/lib/id-primary/lobby.idworld")

    # Operator acts reach the serving node: stop, operator-only wake, start.
    server.succeed(f"id host stop {ROOT} lobby --actor vm-operator")
    assert state("lobby") == "asleep"
    server.succeed(f"id host wake-mode {ROOT} lobby operator")
    server.succeed(f"id host start {ROOT} lobby --actor vm-operator")
    assert state("lobby") == "live"
    seen = json.loads(server.succeed(f"id host inspect {ROOT} lobby --json"))
    assert seen["history"][-1]["actor"] == "vm-operator", seen

    # Archive needs the world asleep, then refuses a start until restored.
    server.fail(f"id host archive {ROOT} lobby")
    server.succeed(f"id host stop {ROOT} lobby")
    server.succeed(f"id host archive {ROOT} lobby")
    server.fail(f"id host start {ROOT} lobby")
    server.succeed(f"id host restore {ROOT} lobby")
    server.succeed(f"id host start {ROOT} lobby")

    # Worlds recorded live come back live after a restart.
    server.succeed(f"id host pin {ROOT} lobby")
    server.systemctl("restart id-primary.service")
    wait_for_node()
    assert state("lobby") == "live"
    seen = json.loads(server.succeed(f"id host inspect {ROOT} lobby --json"))
    assert seen["pinned"] is True, seen
  '';
}
