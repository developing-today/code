tpope/fugitive
mbbill/undotree



require('lazy').setup({
--  'wbthomason/packer.nvim', -- Package manager
  'tpope/vim-fugitive', -- Git commands in nvim
  'tpope/vim-rhubarb', -- Fugitive-companion to interact with github
  'tpope/vim-commentary', -- "gc" to comment visual regions/lines
  'ludovicchabant/vim-gutentags', -- Automatic tags management
  -- UI to select things (files, grep results, open buffers...)
   { 'nvim-telescope/telescope.nvim', dependencies = { { 'nvim-lua/popup.nvim' }, { 'nvim-lua/plenary.nvim' } } },
  'joshdick/onedark.vim', -- Theme inspired by Atom
  'itchyny/lightline.vim', -- Fancier statusline
  -- Add indentation guides even on blank lines
  'lukas-reineke/indent-blankline.nvim',
  -- Add git related info in the signs columns and popups
  --use { 'lewis6991/gitsigns.nvim', requires = { 'nvim-lua/plenary.nvim' } }
  -- Highlight, edit, and navigate code using a fast incremental parsing library
  'nvim-treesitter/nvim-treesitter',
  -- Additional textobjects for treesitter
  'nvim-treesitter/nvim-treesitter-textobjects',
  'neovim/nvim-lspconfig', -- Collection of configurations for built-in LSP client
  --'hrsh7th/nvim-compe', -- Autocompletion plugin
  --'williamboman/nvim-lsp-installer',
   'williamboman/mason.nvim',
    'williamboman/mason-lspconfig.nvim',
  'ray-x/go.nvim',
  'ray-x/guihua.lua',

 ({
   "hrsh7th/nvim-cmp",
     dependencies= {

   "hrsh7th/cmp-buffer",
   "hrsh7th/cmp-nvim-lsp",
   "hrsh7th/cmp-path",
   "hrsh7th/cmp-nvim-lua",
   "saadparwaiz1/cmp_luasnip",
   }
 }),


  'L3MON4D3/LuaSnip', -- Snippets plugin
  'mattn/emmet-vim',
  'jwalton512/vim-blade',
  'alvan/vim-closetag',
  'nvim-lua/completion-nvim',
  'psliwka/vim-smoothie',
  'terryma/vim-multiple-cursors',
 -- 'SirVer/ultisnips',
--  'honza/vim-snippets',
  'hrsh7th/vim-vsnip',
  'hrsh7th/vim-vsnip-integ',
  --'rstacrus/vim-closer',
 -- 'jiangmiao/auto-pairs',
'windwp/nvim-autopairs',	
 {'akinsho/flutter-tools.nvim', dependencies = 'nvim-lua/plenary.nvim'},
  'Neevash/awesome-flutter-snippets',

 {'nvim-telescope/telescope-fzf-native.nvim', build ='make',cond= vim.fn.executable 'make' ==1},
'fatih/vim-go', 

})

 simple lazyloading examples

use{'nvim-telescope/telescope.nvim',cmd='Telescope'}

use{'tommcdo/vim-lion',keys={{'x','gl'},{'n','gl'},{'x','gL'},{'n','gL'}}}

complex lazyloading examples

use{'hrsh7th/nvim-cmp',requires={
    {'hrsh7th/cmp-buffer',after='nvim-cmp'}
}}

use{'glepnir/dashboard-nvim',
    cmd={'Dashboard','DashboardNewFile'},
    setup=function ()
        vim.api.nvim_create_autocmd('Vimenter',{callback=function()
            if vim.fn.argc()==0 and vim.fn.line2byte('$')==-1 then
                vim.cmd'Dashboard'
            end
        end})
    end
}

use{'anuvyklack/pretty-fold.nvim',
    requires='anuvyklack/nvim-keymap-amend',
    config='require "pretty-fold".setup{}',
    event='User isfolded',
    setup=function ()
        local fn=vim.fn
        local fastfoldtimer
        fastfoldtimer=fn.timer_start(2000,function()
            if #fn.filter(fn.range(1,fn.line'$'),'foldlevel(v:val)>0')>0 then
                vim.cmd('doautocmd User isfolded')
                fn.timer_stop(fastfoldtimer)
            end
        end,{['repeat']=-1})
    end
}

use{'elihunter173/dirbuf.nvim',
    cmd={'Dirbuf'},
    setup=function()
        vim.api.nvim_create_autocmd('BufEnter',{
            command="if isdirectory(expand('%')) && !&modified|execute 'Dirbuf'|endif"
        })
    end
}

requires lazyloading examples

use{'rbong/vim-flog't,
    setup=function ()
        vim.api.nvim_create_user_command(cmd,[[
        lua require"packer.load"({"vim-fugitive"},{},_G.packer_plugins)
        require"packer.load"({"vim-flog"},{cmd="Flog",l1=<line1>,l2=<line2>,bang=<q-bang>,args=<q-args>,mods="<mods>"},_G.packer_plugins)]],
            {nargs='*',range=true,bang=true,complete='file'}
        )
    end,
    opt=true
}
use{'tpope/vim-fugitive',
    opt=true
}

rplugin lazyloading examples

vim.g.loaded_remote_plugins="not loaded"
use{'ripxorip/aerojump.nvim',
    config=function ()
        vim.cmd[[
        if g:loaded_remote_plugins=="not loaded"
            unlet g:loaded_remote_plugins
            source /usr/share/nvim/runtime/plugin/rplugin.vim
        endif
        ]]
    end,
    cmd='Aerojump'
}

builtin lazyloading examples

vim.g.loaded_tutor_mode_plugin=1
vim.api.nvim_create_user_command('Tutor',[[
delcommand Tutor
unlet g:loaded_tutor_mode_plugin
source /usr/share/nvim/runtime/plugin/tutor.vim
Tutor <args>]],{nargs='?',complete='custom,tutor#TutorCmdComplete'})


Automatic software installation

Homebrew formulae:

    GNU core utilities
    git
    ag
    bash (latest version)
    bash-completion
    ffmpeg
    graphicsmagick
    jpeg
    macvim
    node
    optipng
    rsync (latest version, rather than the out-dated OS X installation)
    tree
    wget

Node packages:

    gify

Vim plugins:

    ctrlp.vim
    html5.vim
    syntastic
    vim-colors-solarized
    vim-git
    vim-javascript
    vim-markdown
    vim-mustache-handlebars
    vim-pathogen

[submodule "vim/bundle/vim-pathogen"]
	path = vim/bundle/vim-pathogen
	url = git://github.com/tpope/vim-pathogen.git

[submodule "vim/bundle/vim-colors-solarized"]
	path = vim/bundle/vim-colors-solarized
	url = git://github.com/altercation/vim-colors-solarized.git
[submodule "vim/bundle/vim-markdown"]
	path = vim/bundle/vim-markdown
	url = git://github.com/tpope/vim-markdown.git
[submodule "vim/bundle/mustache"]
	path = vim/bundle/mustache
	url = git://github.com/mustache/vim-mustache-handlebars.git
[submodule "vim/bundle/html5"]
	path = vim/bundle/html5
	url = git://github.com/othree/html5.vim.git
[submodule "vim/bundle/vim-git"]
	path = vim/bundle/vim-git
	url = git://github.com/tpope/vim-git.git
[submodule "vim/bundle/syntastic"]
	path = vim/bundle/syntastic
	url = git://github.com/scrooloose/syntastic.git
[submodule "vim/bundle/ctrlp.vim"]
	path = vim/bundle/ctrlp.vim
	url = git://github.com/kien/ctrlp.vim.git
[submodule "vim/bundle/vim-javascript"]
	path = vim/bundle/vim-javascript
	url = git://github.com/pangloss/vim-javascript.git


  bufferline.nvim
cmp-buffer
cmp-git
cmp-nvim-lsp
cmp-path
cmp-spell
cmp-vsnip
copliot.vim
dashboard-nvim
gitsigns.nvim
indent-blankline.nvim
jinja.vim
lspsaga.nvim
lualine.nvim
nginx.vim
nvim-cmp
nvim-lspconfig
nvim-tree.lua
nvim-treesitter
nvim-web-devicons
octo.nvim
plenary.nvim
rust.vim
telescope.nvim
tokyonight.nvim
typescript-vim
vim-better-whitespace
vim-colors-solarized
vim-endwise
vim-fugitive
vim-go
vim-hclfmt
vim-json
vim-jsx-pretty
vim-markdown
vim-mipssyntax
vim-ocaml
vim-protobuf
vim-python-pep8-indent
vim-rhubarb
vim-sensible
vim-surround
vim-systemd
vim-terraform
vim-toml
vim-visual-multi
vim-vsnip
vim-which-key
vim-yaml




update libs to pass args through in cr cre etc
