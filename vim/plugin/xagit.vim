" xagit.vim — open xagit (a Magit-style git TUI) in a floating window.
" Requires Vim 8.2+ / 9.x compiled with +terminal and +popupwin.
" Maintainer: sigmaxue  |  Repository: https://github.com/sigmaxue/xagit

if exists('g:loaded_xagit')
  finish
endif
let g:loaded_xagit = 1

if !has('terminal') || !has('popupwin')
  " Loading the plugin must not break Vim without these features: only the
  " command is defined and it reports the problem when used.
endif

" ---------------------------------------------------------------------------
" Configuration
" ---------------------------------------------------------------------------

" Full path to the xagit executable. When unset, the plugin looks for
" 'xagit' in $PATH and then for a local release/debug build.
if !exists('g:xagit_cmd')
  let g:xagit_cmd = ''
endif

" Fraction of the Vim screen the floating window occupies (0.0 - 1.0).
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
" Helpers
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

" ---------------------------------------------------------------------------
" Open xagit in a popup terminal
" ---------------------------------------------------------------------------

" Only one terminal popup can exist at a time (E861): remember ours so a
" second :Xagit focuses the existing one instead of throwing an error.
let s:popup_id = 0

function! s:XagitFloat() abort
  if !has('terminal') || !has('popupwin')
    echohl ErrorMsg
    echomsg 'xagit.vim needs Vim with +terminal and +popupwin'
    echohl None
    return
  endif

  " Reuse a still-visible popup (xagit is running inside it).
  if s:popup_id && !empty(popup_getpos(s:popup_id))
    call popup_setoptions(s:popup_id, {'zindex': 50})
    return
  endif
  let s:popup_id = 0

  let l:cmd = s:FindCmd()
  if empty(l:cmd) || !executable(l:cmd)
    echohl ErrorMsg
    echomsg 'xagit executable not found. Set g:xagit_cmd to its full path.'
    echohl None
    return
  endif

  let l:dir = s:RepoDir()
  let l:w = max([float2nr(&columns * g:xagit_width),  g:xagit_min_width])
  let l:h = max([float2nr(&lines   * g:xagit_height), g:xagit_min_height])
  let l:w = min([l:w, &columns - 4])
  let l:h = min([l:h, &lines   - 4])
  let l:line = max([1, (&lines   - l:h) / 2])
  let l:col  = max([1, (&columns - l:w) / 2])

  " A terminal shown in a popup must first be created hidden. term_finish
  " closes the popup when xagit exits (C-x C-c).
  let l:buf = term_start([l:cmd, l:dir], {
        \ 'hidden': 1,
        \ 'term_finish': 'close',
        \ 'term_kill': 'kill',
        \ 'term_cols': l:w,
        \ 'term_rows': l:h,
        \ })

  let s:popup_id = popup_create(l:buf, {
        \ 'line': l:line,
        \ 'col': l:col,
        \ 'minwidth': l:w,
        \ 'maxwidth': l:w,
        \ 'minheight': l:h,
        \ 'maxheight': l:h,
        \ 'fixed': 1,
        \ 'border': [],
        \ 'padding': [0, 1, 0, 1],
        \ 'borderchars': ['─', '│', '─', '│', '╭', '╮', '╯', '╰'],
        \ 'title': ' xagit ',
        \ 'zindex': 50,
        \ 'callback': {_, __ -> s:PopupClosed()},
        \ })
endfunction

function! s:PopupClosed() abort
  let s:popup_id = 0
endfunction

command! -bar Xagit call s:XagitFloat()

if !hasmapto('<Plug>Xagit', 'n') && maparg('<leader>g', 'n') ==# ''
  nmap <silent> <leader>g <Plug>Xagit
endif
nnoremap <silent> <Plug>Xagit :Xagit<CR>
