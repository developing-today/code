{ lib, buildGoModule }:

buildGoModule {
  pname = "fleet-placement-record";
  version = "0.1.0";
  src = lib.cleanSource ./.;
  vendorHash = lib.fakeHash;
  subPackages = [ "cmd/placement-record" ];

  postInstall = ''
    mkdir -p $out/share/fleet-placement
    cp ${../../../doc/2026-11-03T15-10-56Z_design_task-placement-framework-v1/placement-policy.yaml} \
      $out/share/fleet-placement/placement-policy.yaml
  '';

  meta = {
    description = "Evaluate and record fleet task placement decisions";
    mainProgram = "placement-record";
    platforms = lib.platforms.linux;
  };
}
