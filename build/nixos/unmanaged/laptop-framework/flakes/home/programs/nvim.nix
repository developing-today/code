{pkgs, ...}: {
  enable = true;
  defaultEditor = true;
  viAlias = true;
  vimAlias = true;
  vimdiffAlias = true;
  package = pkgs.neovim-nightly;

  extraConfig = ''
    require'packer'.startup(function()
      use {
        "neovim/nvim-lspconfig",
        config = function()
          require "plugins.configs.lspconfig"
          -- require "custom.configs.lspconfig"
        end,
      }
      use {
        "lewis6991/gitsigns.nvim",
        config = function(_, opts)
          require("scrollbar.handlers.gitsigns").setup()
          require("gitsigns").setup(opts)
        end,
      }
      use {
        "nvim-telescope/telescope.nvim",
        config = function()
          require "plugins.configs.telescope"
          -- require "custom.configs.telescope"
        end,
      }

      -- ... (continue with other plugins and configurations)
    end)
  '';

  plugins = with pkgs.vimPlugins; [
    nvim-lspconfig
    which-key-nvim
    gitsigns-nvim
    telescope-nvim
    nvim-treesitter
    nvim-cmp
    luasnip
    indent-blankline-nvim
    null-ls-nvim
    vim-fugitive
    vim-polyglot
  ];
}
