{pkgs, ...}: {
  extraConfigLua = builtins.readFile ./extra.lua;
  extraPackages = [pkgs.xclip];
  extraPlugins = with pkgs.vimPlugins; [
    vim-sleuth
  ];
}
