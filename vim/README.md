# xagit.vim

Open [xagit](../README.md) (a Magit-style git TUI) in a **floating window**
from Vim, without leaving your editor.

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

- **Vim 8.2+ / 9.x** with `+terminal` and `+popupwin`
  (check `:echo has('terminal') && has('popupwin')`).
  Neovim is not supported — use `:terminal` / a floating-window plugin there.
- The `xagit` executable (in `$PATH`, or set `g:xagit_cmd`).

## Install

Use a plugin manager, pointing it at the `vim` subdirectory:

```vim
" vim-plug
Plug 'sigmaxue/xagit', { 'rtp': 'vim' }

" Vundle
Plugin 'sigmaxue/xagit', { 'rtp': 'vim' }

" pathogen
" git clone https://github.com/sigmaxue/xagit ~/.vim/bundle/xagit
```

Or with Vim 8 packages (no manager), make `vim/` itself the package:

```sh
mkdir -p ~/.vim/pack/xagit/start
git clone https://github.com/sigmaxue/xagit ~/.vim/xagit-src
ln -s ~/.vim/xagit-src/vim ~/.vim/pack/xagit/start/xagit
```

Simplest: add the directory to your runtimepath (or copy its contents into
`~/.vim/`):

```vim
set runtimepath+=/path/to/xagit/vim
```

Then generate the help tags: `:helptags /path/to/xagit/vim/doc` and read
`:help xagit`.

## Usage

| Command | Action |
|---|---|
| `:Xagit` | open xagit in a floating window |
| `<leader>g` | same (default mapping, only if free) |
| `<Plug>Xagit` | remappable, e.g. `nmap <leader>m <Plug>Xagit` |

The repository is derived from the current file's directory; xagit walks up
to the git toplevel itself, so files in subdirectories work too. Calling
`:Xagit` again while the popup is visible re-focuses it (Vim allows only one
terminal popup at a time).

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
- **Needs `+terminal` / `+popupwin`** — use a full Vim build; alternatively
  `:!xagit` suspends Vim and runs xagit in the shell.
- **Blank/garbled screen** — set `TERM=xterm-256color`, try
  `XAGIT_COLOR=256` or `XAGIT_ASCII=1`.
- **`C-s` does nothing inside xagit** — disable XOFF flow control with
  `stty -ixon` before starting Vim.

Full documentation: `:help xagit`.
