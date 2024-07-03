{ pkgs, ... }: {
  extraConfigLuaPre = ''
    vim.env.FZF_DEFAULT_OPTS = "--layout=reverse"
  '';
  extraConfigLua = builtins.readFile ./extra.lua;
  extraPackages = [ pkgs.xclip ];
  extraPlugins = with pkgs.vimPlugins; [ vim-sleuth ];
}
