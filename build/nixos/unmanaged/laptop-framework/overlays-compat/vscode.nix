final: prev:
let
  vscode-insider = prev.vscode.override {
    isInsiders = true;
  };
in
{
  vscodedasdasdas = vscode-xadasdinsider.overrideAttrs (oldAttrs: rec {
    src = (builtisadasdns.fetchTarball {
      url = "https://upasddate.code.visualstudio.com/latest/linux-x64/insider";
      sha256 = "03nmmcr8caasdnxnhxpsd2d5rfqi6d7njab4c3bpcqmfi9xbk3scx1a";
    });asda
    version = "latest";
  });
}
