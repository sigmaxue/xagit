# xagit.vim

Open [xagit](../README.md) (a Magit-style git TUI) in a **floating window**
from **Vim or Neovim**, without leaving your editor. Same commands and
options on both; the backend is chosen automatically.

- **Vim 8.2+/9.x** (`+terminal +popupwin`): a `popup` running a hidden terminal
- **Neovim 0.5+**: an `nvim_open_win()` floating window running via `termopen()`

```
┌──────────────────────────────────────────────┐
│  xagit  (full TUI inside a popup terminal)   │
│  Unstaged changes (1)                        │
│    modified   src/main.rs                    │
│  Untracked files (1)                         │
│    new.txt                                   │
│  ...                                         │
└──────────────────────────────────────────────┘
```

Vim gives the popup a real terminal, so xagit's raw mode, alternate screen
and key handling work as if it were launched from the shell. Quit xagit with
`C-x C-c` and the popup closes automatically.

## Requirements

- **Vim 8.2+ / 9.x** with `+terminal` **and** `+popupwin`, **or**
  **Neovim 0.5+** (`nvim_open_win()` / `termopen()`).
  Check `:echo has('nvim') || (has('terminal') && has('popupwin'))`.
- The `xagit` executable (in `$PATH`, or set `g:xagit_cmd`).

One `vim/plugin/xagit.vim` (Vimscript) serves both; no Lua file needed.

## Install

The plugin lives in the repo's `vim/` subdirectory. Point your manager's
runtimepath at it.

**Vim** (vim-plug / Vundle / pathogen):

```vim
Plug 'sigmaxue/xagit', { 'rtp': 'vim' }        " vim-plug
Plugin 'sigmaxue/xagit', { 'rtp': 'vim' }     " Vundle
" pathogen: git clone https://github.com/sigmaxue/xagit ~/.vim/bundle/xagit
```

**Neovim** (lazy.nvim, `~/.config/nvim/lua/plugins/xagit.lua`):

```lua
return {
  'sigmaxue/xagit',
  init = function()
    -- the plugin is in vim/, not the repo root
    vim.opt.rtp:append(vim.fn.stdpath('data') .. '/lazy/xagit/vim')
    vim.g.xagit_cmd = '/path/to/xagit/target/release/xagit' -- optional
    vim.cmd('runtime! plugin/xagit.vim')
  end,
}
```

Or with **Vim 8 / Neovim packages** (no manager), make `vim/` itself the
package:

```sh
git clone https://github.com/sigmaxue/xagit ~/.local/share/xagit-src
# Vim
mkdir -p ~/.vim/pack/xagit/start
ln -s ~/.local/share/xagit-src/vim ~/.vim/pack/xagit/start/xagit
# Neovim
mkdir -p ~/.local/share/nvim/site/pack/xagit/start
ln -s ~/.local/share/xagit-src/vim ~/.local/share/nvim/site/pack/xagit/start/xagit
```

Simplest for both: add the dir to runtimepath
(`set rtp+=/path/to/xagit/vim` / `vim.opt.rtp:append('/path/to/xagit/vim')`),
then `:helptags /path/to/xagit/vim/doc` and read `:help xagit`.

## Usage

| Command | Action |
|---|---|
| `:Xagit` | open xagit in a floating window |
| `<leader>g` | same (default mapping, only if free) |
| `<Plug>Xagit` | remappable, e.g. `nmap <leader>m <Plug>Xagit` |

The repository is derived from the current file's directory; xagit walks up
to the git toplevel itself, so files in subdirectories work too. Calling
`:Xagit` again while the float is visible re-focuses it on both editors
(Vim allows only one terminal popup at a time).

Inside xagit: `h`/`?` dispatch, `s`/`u` stage/unstage, `c c` commit,
`g` refresh, `q` back, `C-x C-c` quit.

## Configuration

Set these in your vimrc **before** the plugin loads:

```vim
let g:xagit_cmd         = '/path/to/xagit/target/release/xagit' " '' = search $PATH / ../target
let g:xagit_width       = 0.9        " fraction of &columns
let g:xagit_height      = 0.85       " fraction of &lines
let g:xagit_min_width   = 60
let g:xagit_min_height  = 18
let g:xagit_dir         = ''         " '' = current file's dir; or a string, or a Funcref -> path
```

xagit's own environment variables (`XAGIT_COLOR`, `XAGIT_THEME`,
`XAGIT_ASCII`, `NO_COLOR`, `TERM`) apply as usual.

## Troubleshooting

- **"xagit executable not found"** — set `g:xagit_cmd` to its absolute path
  or install it into `$PATH`.
- **Needs `+terminal` / `+popupwin` (Vim) or Neovim 0.5+** — use a full Vim
  build or a recent Neovim; alternatively `:!xagit` suspends the editor and
  runs xagit in the shell.
- **Neovim: `:Xagit` unknown command** — ensure `vim/` is on `runtimepath`
  and the file was sourced (`runtime! plugin/xagit.vim`); see the lazy.nvim
  example above.
- **Blank/garbled screen** — set `TERM=xterm-256color`, try
  `XAGIT_COLOR=256` or `XAGIT_ASCII=1`.
- **`C-s` does nothing inside xagit** — disable XOFF flow control with
  `stty -ixon` before starting Vim.

Full documentation: `:help xagit`.
