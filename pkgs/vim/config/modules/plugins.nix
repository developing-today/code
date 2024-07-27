{ ... }:
{
  plugins = {
    #telescope-tabs = {};
    telescope = { };
    #goto-preview = {};
    #coc = {};
    bufferline = { };
    lightline = { };
    commentary = { };
    illuminate = { };
    fugitive = { };
    #which-key.window.winblend = 10;
    nvim-colorizer = { };
    quickmath = { };
    surround = { };
    notify = {
      level = 2;
      topDown = false;
      maxWidth = 400;
    };
    dap.extensions = {
      dap-ui.enable = true;
      dap-virtual-text.enable = true;
    };
    inc-rename = { };
    #neoscroll = {};
    nix = { };
    lsp-format.setup.typescript = {
      order = [ "null-ls" ];
      exclude = [
        "tsserver"
        "eslint"
      ];
    };
    cmp = {
      autoEnableSources = true;
      # see -> https://github.com/pta2002/nixvim/blob/main/plugins/completion/nvim-cmp/cmp-helpers.nix#L12
      #sources = [
      #  {name = "luasnip";}
      #  {name = "nvim_lsp";}
      #  {name = "nvim_lua";}
      #  {name = "path";}
      #  {name = "buffer";}
      #];
      # see -> https://github.com/hrsh7th/nvim-cmp/blob/main/lua/cmp/config/mapping.lua#L36
      #mappingPresets = ["insert" "cmdline"];
      settings = {
        mapping = {
          "<C-b>" = ''cmp.mapping(cmp.mapping.scroll_docs(-1), { "i" })'';
          "<C-f>" = ''cmp.mapping(cmp.mapping.scroll_docs(1), { "i" })'';
          "<C-e>" = "cmp.mapping(cmp.mapping.abort())";
          "<C-l>" = "cmp.mapping(cmp.mapping.complete())";
          "<CR>" = "cmp.mapping.confirm { select = true }";
        };
        #snippet.expand = "luasnip";
        formatting.fields = [
          "kind"
          "abbr"
          "menu"
        ];
        window = {
          completion = {
            winhighlight = "Normal:Pmenu,FloatBorder:Pmenu,Search:None";
            border = "single";
          };
          documentation = {
            winhighlight = "Normal:Pmenu,FloatBorder:Pmenu,Search:None";
            border = "single";
          };
        };
      };
    };
    mini = { };
    lspkind = { };
    treesitter = { };
    harpoon.keymaps = {
      addFile = "<leader>a";
      toggleQuickMenu = "<leader>s";
      navFile = {
        "1" = "<C-j>";
        "2" = "<C-k>";
        "3" = "<C-l>";
        "4" = "<C-m>";
      };
    };
    gitsigns = { };
    lualine = { };
    telescope = { };
    #copilot-lua = {};
    copilot-vim = { };
    lsp = {
      servers = {
        bashls = { };
        gopls = { };
        #rnix-lsp = {};
        rust-analyzer = { };
        html = { };
        eslint = { };
        tsserver = { };
        lua-ls = { };
        jsonls = { };
        pylsp = { };
        #pyright = {};
      };
      onAttach = ''
        -- Enable completion triggered by <c-x><c-o>
        vim.api.nvim_buf_set_option(bufnr, 'omnifunc', 'v:lua.vim.lsp.omnifunc')

        -- Mappings.
        -- See `:help vim.lsp.*` for documentation on any of the below functions
        local bufopts = { noremap=true, silent=true, buffer=bufnr }
        vim.keymap.set('n', 'gD', vim.lsp.buf.declaration, bufopts)
        vim.keymap.set('n', 'gd', vim.lsp.buf.definition, bufopts)
        vim.keymap.set('n', 'K', vim.lsp.buf.hover, bufopts)
        vim.keymap.set('n', 'gi', vim.lsp.buf.implementation, bufopts)
        vim.keymap.set('n', '<C-k>', vim.lsp.buf.signature_help, bufopts)
        vim.keymap.set('n', '<space>wa', vim.lsp.buf.add_workspace_folder, bufopts)
        vim.keymap.set('n', '<space>wr', vim.lsp.buf.remove_workspace_folder, bufopts)
        vim.keymap.set('n', '<space>wl', function()
        print(vim.inspect(vim.lsp.buf.list_workspace_folders()))
        end, bufopts)
        vim.keymap.set('n', '<space>D', vim.lsp.buf.type_definition, bufopts)
        vim.keymap.set('n', '<space>rn', vim.lsp.buf.rename, bufopts)
        vim.keymap.set('n', '<space>ca', vim.lsp.buf.code_action, bufopts)
        vim.keymap.set('n', '<space>cf', function() vim.lsp.buf.format {
          filter = function(client)
          -- Disable tsserver format
            if client.name == "tsserver" then
              return false
            end
            return true
          end,
          bufnr = bufnr,
        } end, bufopts)
        vim.keymap.set('n', 'gr', vim.lsp.buf.references, bufopts)

        -- Mappings.
        -- See `:help vim.diagnostic.*` for documentation on any of the below functions
        local opts = { noremap=true, silent=true }
        vim.keymap.set('n', '<space>e', vim.diagnostic.open_float, opts)
        vim.keymap.set('n', '[d', vim.diagnostic.goto_prev, opts)
        vim.keymap.set('n', ']d', vim.diagnostic.goto_next, opts)
        vim.keymap.set('n', '<space>q', vim.diagnostic.setloclist, opts)
      '';
    };
    # TODO: https://github.com/charm-community/freeze.nvim
    none-ls.sources = {
      formatting = {
        prettier.enable = true;
        # eslint.enable = true;
        shfmt.enable = true;
      };
      #diagnostics.shellcheck.enable = true;
    };
    nix = { };
    nvim-autopairs = { };
    surround = { };
    nvim-tree.updateFocusedFile = {
      enable = true;
    };
  };
}
