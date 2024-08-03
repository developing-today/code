{ pkgs, ... }:
let
  plugins-repo = pkgs.fetchFromGitHub {
    owner = "yazi-rs";
    repo = "plugins";
    rev = "06e5fe1c7a2a4009c483b28b298700590e7b6784";
    sha256 = "jg8+GDsHOSIh8QPYxCvMde1c1D9M78El0PljSerkLQc=";
  };
in
{
  programs.yazi = {
    enable = true;
    enableZshIntegration = true;
    shellWrapperName = "y";

    settings = {
      manager = {
        show_hidden = true;
      };
      preview = {
        max_width = 1000;
        max_height = 1000;
      };
    };

    plugins = {
      chmod = "${plugins-repo}/chmod.yazi";
      full-border = "${plugins-repo}/full-border.yazi";
      max-preview = "${plugins-repo}/max-preview.yazi";
      /*
        starship = pkgs.fetchFromGitHub {
        				owner = "Rolv-Apneseth";
        				repo = "starship.yazi";
        				rev = "0a141f6dd80a4f9f53af8d52a5802c69f5b4b618";
        				sha256 = "jg8+GDsHOSIh8QPYxCvMde1c1D9M78El0PljSerkLQc=";
        			};
      */
    };

    initLua = ''
      	require("full-border"):setup()
    '';
    # 		require("starship"):setup()
    keymap = {
      manager.prepend_keymap = [
        {
          on = [ "T" ];
          run = "plugin --sync max-preview";
          desc = "Maximize or restore the preview pane";
        }
        {
          on = [
            "c"
            "m"
          ];
          run = "plugin chmod";
          desc = "Chmod on selected files";
        }
      ];
    };
  };
}
