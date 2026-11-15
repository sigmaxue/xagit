" xagit.vim — open xagit (a Magit-style git TUI) in a floating window.
" Works with Vim 8.2+/9.x (+terminal +popupwin) AND Neovim 0.5+
" (nvim_open_win / termopen). Same command and options on both.
" Maintainer: sigmaxue  |  Repository: https://github.com/sigmaxue/xagit

if exists('g:loaded_xagit')
  finish
endif
let g:loaded_xagit = 1

" ---------------------------------------------------------------------------
" Configuration (shared by Vim and Neovim)
" ---------------------------------------------------------------------------

" Full path to the xagit executable. When unset, look for 'xagit' in $PATH
" and then for a local release/debug build next to this plugin.
if !exists('g:xagit_cmd')
  let g:xagit_cmd = ''
endif

" Fraction of the editor screen the floating window occupies (0.0 - 1.0).
if !exists('g:xagit_width')
  let g:xagit_width = 0.9
endif
if !exists('g:xagit_height')
  let g:xagit_height = 0.85
endif

" Minimum size, in case the terminal is small.
if !exists('g:xagit_min_width')
  let g:xagit_min_width = 60
endif
if !exists('g:xagit_min_height')
  let g:xagit_min_height = 18
endif

" Repository directory. Empty = derive from the current file's directory
" (xagit walks up to the git toplevel itself). A string is expanded and used
" verbatim; a Funcref is called with no arguments and must return a path.
if !exists('g:xagit_dir')
  let g:xagit_dir = ''
endif

" ---------------------------------------------------------------------------
" Helpers (shared)
" ---------------------------------------------------------------------------

function! s:FindCmd() abort
  if !empty(g:xagit_cmd)
    return g:xagit_cmd
  endif
  if executable('xagit')
    return exepath('xagit')
  endif
  " Fall back to a build inside the plugin's repository:
  "   <repo>/vim/plugin/xagit.vim -> <repo>/target/{release,debug}/xagit
  let l:root = fnamemodify(expand('<sfile>:p:h:h:h'), ':p')
  for l:cand in [l:root . 'target/release/xagit', l:root . 'target/debug/xagit']
    if executable(l:cand)
      return l:cand
    endif
  endfor
  return ''
endfunction

function! s:RepoDir() abort
  if type(g:xagit_dir) == v:t_func
    return call(g:xagit_dir, [])
  endif
  if type(g:xagit_dir) == v:t_string && !empty(g:xagit_dir)
    return expand(g:xagit_dir)
  endif
  let l:dir = expand('%:p:h')
  return empty(l:dir) ? getcwd() : l:dir
endfunction

function! s:Geometry() abort
  let l:w = max([float2nr(&columns * g:xagit_width),  g:xagit_min_width])
  let l:h = max([float2nr(&lines   * g:xagit_height), g:xagit_min_height])
  let l:w = min([l:w, &columns - 4])
  let l:h = min([l:h, &lines   - 4])
  return {
        \ 'w': l:w,
        \ 'h': l:h,
        \ 'row': max([0, (&lines   - l:h) / 2]),
        \ 'col': max([0, (&columns - l:w) / 2]),
        \ }
endfunction

function! s:Supported() abort
  if has('nvim')
    return exists('*nvim_open_win') && exists('*termopen')
  endif
  return has('terminal') && has('popupwin')
endfunction

" ---------------------------------------------------------------------------
" Vim backend: a hidden terminal shown in a popup window (:help popup-terminal)
" ---------------------------------------------------------------------------

" Only one terminal popup can exist at a time (E861): remember ours so a
" second :Xagit focuses it instead of throwing an error.
let s:popup_id = 0

function! s:PopupClosed() abort
  let s:popup_id = 0
endfunction

function! s:OpenVim(l, dir, g) abort
  if s:popup_id && !empty(popup_getpos(s:popup_id))
    call popup_setoptions(s:popup_id, {'zindex': 50})
    return
  endif
  let s:popup_id = 0

  " A terminal shown in a popup must first be created hidden.
  " term_finish closes the popup when xagit exits (C-x C-c).
  let l:buf = term_start([a:l, a:dir], {
        \ 'hidden': 1,
        \ 'term_finish': 'close',
        \ 'term_kill': 'kill',
        \ 'term_cols': a:g.w,
        \ 'term_rows': a:g.h,
        \ })

  let s:popup_id = popup_create(l:buf, {
        \ 'line': a:g.row + 1,
        \ 'col': a:g.col + 1,
        \ 'minwidth': a:g.w,
        \ 'maxwidth': a:g.w,
        \ 'minheight': a:g.h,
        \ 'maxheight': a:g.h,
        \ 'fixed': 1,
        \ 'border': [],
        \ 'padding': [0, 1, 0, 1],
        \ 'borderchars': ['─', '│', '─', '│', '╭', '╮', '╯', '╰'],
        \ 'title': ' xagit ',
        \ 'zindex': 50,
        \ 'callback': {_, __ -> s:PopupClosed()},
        \ })
endfunction

" ---------------------------------------------------------------------------
" Neovim backend: a scratch buffer in a floating window, run via termopen
" ---------------------------------------------------------------------------

" Track the open window/buffer so a second :Xagit focuses it and on_exit
" closes the float when xagit quits.
let s:nvim_win = 0
let s:nvim_buf = 0

function! s:CloseNvim() abort
  if s:nvim_win && nvim_win_is_valid(s:nvim_win)
    call nvim_win_close(s:nvim_win, v:true)
  endif
  let s:nvim_win = 0
  let s:nvim_buf = 0
endfunction

function! s:OpenNvim(l, dir, g) abort
  " Reuse a still-valid floating window.
  if s:nvim_win && nvim_win_is_valid(s:nvim_win)
    call nvim_set_current_win(s:nvim_win)
    return
  endif
  let s:nvim_win = 0

  let l:buf = nvim_create_buf(v:false, v:true)
  let l:opts = {
        \ 'relative': 'editor',
        \ 'width': a:g.w,
        \ 'height': a:g.h,
        \ 'row': a:g.row,
        \ 'col': a:g.col,
        \ 'border': 'rounded',
        \ 'title': ' xagit ',
        \ 'title_pos': 'center',
        \ 'style': 'minimal',
        \ 'zindex': 50,
        \ }
  let l:win = nvim_open_win(l:buf, v:true, l:opts)
  let s:nvim_win = l:win
  let s:nvim_buf = l:buf

  " Make the float a terminal running xagit. on_exit closes the window.
  call termopen([a:l, a:dir], {
        \ 'on_exit': {_, __, ___ -> s:CloseNvim()},
        \ })
  startinsert
endfunction

" ---------------------------------------------------------------------------
" Entry point
" ---------------------------------------------------------------------------

function! s:XagitFloat() abort
  if !s:Supported()
    echohl ErrorMsg
    echomsg 'xagit.vim needs Vim (+terminal +popupwin) or Neovim 0.5+'
    echohl None
    return
  endif

  let l:cmd = s:FindCmd()
  if empty(l:cmd) || !executable(l:cmd)
    echohl ErrorMsg
    echomsg 'xagit executable not found. Set g:xagit_cmd to its full path.'
    echohl None
    return
  endif

  let l:dir = s:RepoDir()
  let l:g = s:Geometry()
  if has('nvim')
    call s:OpenNvim(l:cmd, l:dir, l:g)
  else
    call s:OpenVim(l:cmd, l:dir, l:g)
  endif
endfunction

command! -bar Xagit call s:XagitFloat()

if !hasmapto('<Plug>Xagit', 'n') && maparg('<leader>g', 'n') ==# ''
  nmap <silent> <leader>g <Plug>Xagit
endif
nnoremap <silent> <Plug>Xagit :Xagit<CR>
