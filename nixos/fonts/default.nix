{ pkgs, ... }:
{
  fonts = {
    packages = with pkgs; [
      # only desktops not servers?
      noto-fonts
      noto-fonts-cjk-sans
      noto-fonts-emoji
      font-awesome
      source-han-sans
      source-han-sans-japanese
      source-han-serif-japanese
      ] ++ builtins.filter lib.attrsets.isDerivation (builtins.attrValues pkgs.nerd-fonts)
    ; # missing other fonts
    fontconfig = {
      # ligatures just give me ligatures what is this
      enable = true;
      defaultFonts = {
        monospace = [ "Meslo LG M Regular Nerd Font Complete Mono" ];
        serif = [
          "Noto Serif"
          "Source Han Serif"
        ];
        sansSerif = [
          "Noto Sans"
          "Source Han Sans"
        ];
      };
    };
  };
}
