# xagit.el

[← 返回仓库](../README.md) · 从 **Emacs** 里用 `M-x xagit` 在**浮动 / 独占窗口**中打开
[xagit](../README.md)（Magit 风格的 git TUI），退出后自动恢复窗口布局。

与 [vim/](../vim/README.md) 插件并列，两者共用同一个 xagit 二进制。

## 特点

- **零外部依赖**：只依赖 Emacs 自带的 `term.el`（Emacs 25.1+）。
- **真·浮动（可选）**：GUI 下若安装了 [`posframe`](https://github.com/tumashu/posframe)，
  用浮动 child frame；否则（以及终端 Emacs `-nw` 下）用一个接近全屏的独占窗口，
  退出后 `set-window-configuration` 恢复原布局。
- **按键原样透传**：把 term 的 escape 键从 `C-c` 挪到 `C-^`，于是 xagit 的
  `C-c C-c`（提交/保存）、`C-c C-k`（取消）、`C-x C-c`（退出）、`C-g`（中止）都能收到；
  要用 Emacs 的 term 命令时按 `C-^` 前缀（如 `C-^ b` 切 buffer）。
- **自动清理**：xagit 进程退出（sentinel）时关闭窗口/frame 并杀掉 term buffer。
- 已在 Emacs 30.2 的终端（`-nw`）里用真实 PTY 实测：TUI 渲染、按键、
  `C-x C-c` 退出均正常。

## 安装

把 `emacs/` 加入 `load-path` 并 require：

```elisp
(add-to-list 'load-path "/path/to/xagit/emacs")
(require 'xagit)
(global-set-key (kbd "C-c x g") #'xagit)
```

`use-package`（straight / elpaca / quelpa）：

```elisp
(use-package xagit
  :load-path "/path/to/xagit/emacs"      ; 或 :straight (:local-repo ...)
  :commands (xagit xagit-quit)
  :bind ("C-c x g" . xagit)
  :custom
  (xagit-executable "/path/to/xagit/target/release/xagit"))
```

`package-vc-install` / `straight` 从 GitHub 安装时，注意本文件在仓库的 `emacs/`
子目录（不是仓库根），需指定 `:lisp-dir "emacs"` 或把 `emacs/` 加进 `load-path`。

## 用法

| 命令 | 作用 |
|---|---|
| `M-x xagit` | 打开（已运行时聚焦） |
| `M-x xagit-quit` | 关闭 xagit 窗口与 buffer |

仓库目录默认取当前 buffer 的 `default-directory`，xagit 会自行
`git rev-parse --show-toplevel` 向上找到 git 根，所以在子目录打开文件也没问题。

xagit 内部：`h`/`?` 命令总览、`s`/`u` 暂存/取消、`c c` 提交、`g` 刷新、
`C-x C-c` 退出（自动关窗）。

## 配置（`M-x customize-group RET xagit`）

| 变量 | 默认 | 说明 |
|---|---|---|
| `xagit-executable` | `"xagit"` | 二进制；绝对路径或在 `exec-path` 中查找；再回退到仓库 `target/{release,debug}/xagit` |
| `xagit-directory` | `nil` | 启动目录；`nil`=当前 buffer 目录，字符串或返回目录的函数 |
| `xagit-width` | `0.9` | 占 frame 宽度比例 |
| `xagit-height` | `0.85` | 占 frame 高度比例 |
| `xagit-posframe` | `t` | GUI 且装有 posframe 时用浮动 frame |
| `xagit-buffer-name` | `"*xagit*"` | term buffer 名 |
| `xagit-term-escape-key` | `?\C-^` | term escape 键（避开 xagit 用的 `C-c`） |

示例：始终在 git 根打开（配合 `magit`/`vc` 或纯 git 调用）：

```elisp
(setq xagit-directory
      (lambda ()
        (or (when (fboundp 'magit-toplevel) (magit-toplevel))
            (locate-dominating-file default-directory ".git")
            default-directory)))
```

xagit 自身的环境变量（`XAGIT_COLOR`、`XAGIT_THEME`、`XAGIT_ASCII`、`TERM`）照常生效；
在 GUI 的 `term` 里若配色异常，可设 `XAGIT_COLOR=256` 或 `XAGIT_ASCII=1`。

## 排错

- **"xagit executable not found"** — 设置 `xagit-executable` 为绝对路径，或把 xagit 装进 `exec-path`。
- **按 `C-x C-c` 退出了 Emacs** — 说明 term 在 line/escape 模式；本插件默认 char-mode + 透传，
  若被其它配置改回，执行 `M-x term-char-mode` 或检查 `xagit-term-escape-key`。
- **窗口没恢复 / 残留 `*xagit*` buffer** — `M-x xagit-quit` 手动清理。
- **GUI 下想真浮动** — 安装 posframe 并确认 `xagit-posframe` 非 nil；终端 Emacs 无 child frame，
  自动用独占窗口。
- **`C-s` 在 xagit 里无反应** — 部分终端把 `C-s` 用于 XOFF 流控，启动 Emacs 前 `stty -ixon`。

## 关于按键透传的说明

Emacs `term` 默认拿 `C-c` 当 escape 前缀、且 `C-x` 也是前缀，会吞掉 xagit 的核心键。
本插件为该 term buffer 装一份 `term-raw-map` 的拷贝，把 `C-c`、`C-x`、`C-g` 直接绑定到
`term-send-raw`（发原始控制字节），并把 escape 前缀改到 `C-^`。因此 xagit 收到的
`C-c C-c` 就是两个 `\x03` 字节——正是它内建编辑器 buffer 用来「提交/保存」的组合。
