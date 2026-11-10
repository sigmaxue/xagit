# xagit — Magit-style git TUI in Rust

xagit 是用 Rust 实现的终端 Git 界面，参考 [Magit](https://github.com/magit/magit) 的操作流程、快捷键和 transient 菜单。它结合 Git 命令行操作与 libgit2 只读后端，无需 Emacs 即可查看仓库状态并执行常用 Git 操作。

项目仓库：[sigmaxue/xagit](https://github.com/sigmaxue/xagit)。本项目不是 Magit 的完整替代，兼容范围与限制见下文。

## 目录
- [界面](#界面)
- [特性](#特性)
- [要求](#要求)
- [安装与构建](#安装与构建)
- [快速开始](#快速开始)
- [界面与终端](#界面与终端)
- [常用快捷键与多键解释](#常用快捷键与多键解释)
- [测试与开发](#测试与开发)
  - [libgit2 只读查询层](#libgit2-只读查询层)
- [Magit 兼容范围与已知限制](#magit-兼容范围与已知限制)
- [贡献](#贡献)
- [致谢](#致谢)
- [许可证](#许可证)

## 界面
<img width="1723" height="892" alt="image" src="https://github.com/user-attachments/assets/943a2818-0761-4e50-9b8a-7d57461ff78d" />

<img width="1721" height="888" alt="image" src="https://github.com/user-attachments/assets/16066043-1b13-4331-b503-f80d9b64d2f4" />

## 特性

- 状态视图支持 section（章节）、文件、hunk（差异块）及 region（选中区域）操作，包括暂存、取消暂存、丢弃、应用与反向应用。可用操作取决于当前视图及变更类型。
- 提供分支、提交、远端、fetch/pull/push、merge/rebase、tag、stash、reset、cherry-pick 与 revert 菜单；进行中操作按仓库状态提供 continue/skip/abort 等入口。
- 支持 diff、log、refs、blame、补全、历史选择及 transient 参数；支持多键序列与 Ctrl/Meta 按键。
- 包含 bisect、worktree、submodule、subtree、notes、patch/am 与分支配置功能，以及进程日志视图。
  

  


## 要求

- Rust 工具链与 Cargo：项目使用 Rust 2021 edition，未声明最低 Rust 版本（MSRV）。建议使用当前稳定版。
- Git 命令行程序需位于 `PATH`；提交前需配置 Git 用户身份，远端操作需自行配置认证。
- 支持 Linux 与 macOS 的交互式终端（终端能力与配色见[界面与终端](#界面与终端)）；Windows 未验证。
- `git2` 依赖涉及本地 C 代码构建，需要可用的 C 编译工具链；当前配置关闭默认特性，不要求系统安装 libgit2/OpenSSL。
- 需要手动编辑 rebase todo 或提交消息时，配置可用的外部编辑器；Git 按 `GIT_SEQUENCE_EDITOR`、`GIT_EDITOR`、`core.editor` 等规则选择编辑器。
- `r m`、`r k` 及部分 cherry 搬移操作使用 Perl 脚本改写 todo，需要 `perl`。

## 安装与构建

## 构建

从源码构建：

```bash
git clone https://github.com/sigmaxue/xagit.git
cd xagit
cargo build --release
```

构建产物为 `target/release/xagit`（Windows 通常为 `xagit.exe`）。如需安装到 Cargo 的可执行文件目录：

```bash
cargo install --path .
```

确保 Cargo 的可执行文件目录在 `PATH` 中后，可直接运行 `xagit`。

## 快速开始

在源码目录构建后：

```bash
./target/release/xagit                 # 查看当前仓库
./target/release/xagit /path/to/repo   # 查看指定仓库
```

安装后也可在任意 Git 仓库内运行 `xagit`，或运行 `xagit /path/to/repo`。CLI 只读取第一个位置参数作为路径，省略时使用当前目录；没有专用的 `--help`/`--version` 参数解析。

1. 按 `h` 或 `?` 查看总菜单，按 `n`/`p` 在章节间移动，方向键在行间移动。
2. 按 `TAB` 展开变更；在文件或 hunk 上按 `s` 暂存，按 `u` 取消暂存。
3. 依次按 `c c` 创建提交并在提示行输入消息；按 `g` 刷新。
4. 按 `q` 返回上一视图，状态页中按 `q` 不会退出；按 `C-x C-c` 退出程序。

注意：`k` 是丢弃变更，不是向上移动。hard reset、强制推送、删除与历史改写等操作可能丢失数据，请检查目标、提示和参数，必要时先备份。

## 界面与终端

界面自上而下分为：标题栏（`xagit` 标记、当前视图，右侧为仓库名与 HEAD）、正文、transient 弹窗（以分隔线与正文隔开）、状态栏（视图模式、消息、光标位置）和按键提示栏（按宽度显示能放下的提示）。

- 正文沿用 Magit 的语义配色：区块标题、hash、当前/本地/远程分支、标签、文件状态（modified/new file/deleted…）、hunk 头和增删行各有颜色；日志图按列着色。
- 光标所在行有背景和左侧光标条，光标所在 section 整体浅色高亮（magit-section-highlight），region 另有选区底色。
- 子 section 按层级缩进；`Head:`/`Merge:`/`Author:` 等头部信息保持表格对齐。
- 中文、日文、韩文与 emoji 按两列宽计算：截断、右侧边栏（作者/时间）和弹窗多列都按显示宽度对齐；tab 按 8 列展开，控制字符显示为 `^M` 这类记法。
- 不覆盖终端自身的背景色和前景色，只给高亮、差异行和两条栏着色。每帧整行重绘并使用同步更新，避免闪烁；窗口大小变化后自动重新布局；程序异常退出时也会恢复终端。
- 操作不阻塞界面：每条命令后的缓冲区刷新在后台线程进行，按键立即生效，刷新完成后光标留在原 section；刷新期间仍可用 `n`/`p`/`C-n`/`C-p`/`M-n`/`M-p` 移动，其他命令会先等刷新完成，保证作用于最新内容。fetch/pull/push 等远程操作作为后台进程运行（不接管终端、不弹提示），状态栏显示转动的指示和正在运行的命令，完成后显示结果并刷新。若远程操作需要输入密码、口令或确认主机密钥，会交还终端后重新运行一次以便输入。有远程操作未结束时退出，会询问是否终止它们。

终端能力按环境变量自动检测，Linux 与 macOS 规则相同：

| 能力 | 检测 | 说明 |
| --- | --- | --- |
| 24 位真彩色 | `COLORTERM=truecolor`/`24bit`，或 iTerm2、WezTerm、kitty、Ghostty、Alacritty、foot、VS Code、VTE 系终端（GNOME Terminal 等） | 完整配色 |
| 256 色 | `TERM=*-256color`、`xterm*`、`screen*`/`tmux*`，以及 macOS 自带 Terminal.app | 颜色就近映射到 256 色 |
| 16 色 | Linux 控制台（`TERM=linux`）等 | 不使用背景底色，栏与选区改用反显 |
| 无色 | `NO_COLOR`（任意非空值）或 `TERM=dumb` | 只用粗体、反显、下划线区分 |

可用环境变量覆盖检测结果：

- `XAGIT_COLOR=truecolor|256|16|never`：指定色深（优先于 `NO_COLOR`）。
- `XAGIT_THEME=dark|light`：选择深色或浅色配色。未设置时依次参考 `COLORFGBG`；在 macOS Terminal.app 中跟随系统外观（其默认描述文件随系统深浅色切换）；否则使用深色。
- `XAGIT_ASCII=1`：使用 ASCII 字形（`>`、`v`、`|`、`-`）代替 `▸ ▾ ▌ ─`。Linux 控制台和 Linux 上非 UTF-8 的 locale（如 `LANG=C`）会自动使用 ASCII。

macOS 提示：Meta 键（如 `M-w`、`M-x`）需要终端把 Option 作为 Meta 发送。Terminal.app：设置 → 描述文件 → 键盘 → 勾选「将 Option 键用作 Meta 键」；iTerm2：Profiles → Keys → Left/Right Option key 设为 `Esc+`。在 tmux 中使用真彩色时，需要让 tmux 透传（如 `set -as terminal-features ",*:RGB"`），或设置 `XAGIT_COLOR=256`。

## 常用快捷键与多键解释

### 全局按键（magit-mode-map）

| 键 | 命令 | 说明 |
|----|------|------|
| `h` / `?` | magit-dispatch | 弹出总菜单 |
| `g` | magit-refresh | 刷新状态缓冲区 |
| `q` / `Esc` | bury-buffer | 退出文本视图 / 返回上一视图；状态页不会退出（`C-u C-u q` 关闭其他所有视图，回到状态页） |
| `C-x C-c` | save-buffers-kill-terminal | 退出程序 |
| `TAB` | toggle-section | 折叠/展开 section |
| `RET` | visit-thing | 查看光标处内容（diff / stash / 文件） |
| `n` / `p`；`C-n` / `C-p` / 方向键 | 移动 | 前者按 section 移动，后者按行移动（Magit 没有 vi 式移动：`k` 是 discard，`j` 是跳转） |
| `j` | status-jump | status 中打开 Jump to 菜单：`z` Stashes、`n` Untracked files、`u` Unstaged changes、`s` Staged changes、`fu` / `fp` Unpulled from upstream / push-remote、`pu`（无上游或 HEAD 已在上游中时显示 Recent commits，否则 Unmerged into upstream）/ `pp` Unpushed to upstream / push-remote（section 不存在时显示为括号内的不可用项，按下只提示 `Inapt command`；带 `C-u` 时同时展开该 section 并置于窗口顶部）、`j` Imenu（先选顶层 section，再选其中一项）。diff / revision buffer 中跳转到 diffstat 或 diff。dispatch 中的 `j` 在 status 里是 Jump to section，在其他 buffer 里是 Display status |
| `^` | section-up | 跳到所在 section 的标题行 |
| `s` / `u` | stage / unstage | 暂存 / 取消暂存光标处 section、文件、hunk 或选中 region |
| `S` / `U` | stage-modified / unstage-all | 全部暂存 / 全部取消暂存 |
| `k` | discard | 丢弃光标处变更（带确认） |
| `a` / `v` | apply / reverse | 把光标处变更（或 stash）应用到工作区 / 在工作区反向应用 |
| `K` | file-untrack | `git rm --cached` |
| `x` | reset-quickly | `git reset --mixed`（前缀参数为 --hard） |
| `:` / `Q` | git-command | 读取完整命令行（初始输入 `git `），在启动目录执行 |
| `!` | run 菜单 | `!`/`p` 在仓库根目录/启动目录运行 git 命令，`s`/`S` 运行任意 shell 命令，`k`/`a`/`b` 启动 gitk（默认/--all/--branches），`g` 启动 git gui，`m` 用 mergetool 解决冲突 |
| `$` | process-buffer | 显示最近执行的 git 命令（含退出码与 stderr，失败行以 `!` 开头；查询类命令不记录） |
| `B` / `Z` / `%` / `o` / `O` / `T` | bisect / worktree / worktree / submodule / subtree / notes | 对应菜单 |
| `e` / `E` | ediff dwim / ediff 菜单 | 文本 diff 视图；`E s` 用 `git add -p` 选择性暂存，`E m`/`E M` 用 `git mergetool` 解决冲突，`E t` 用 `git mergetool --gui`；`! m` 与 `E t` 相同 |
| `D` / `L` | diff-refresh / log-refresh | 修改当前 diff / log 视图的参数，`g` 按新参数重新显示 |
| `V` | revert 菜单 | `V` 生成 revert 提交、`v` 只在工作区反向应用 |
| `H` / `J` / `Y` | section info / display buffer / cherries | 当前行信息、状态缓冲区文本、cherry 视图 |
| `w` / `W` | am / patch 菜单 | `git am` 菜单；patch 菜单（创建 patch、应用 patch、保存 patch、request-pull） |
| `b C` / `l r` | branch configure / reflog | 分支配置菜单与 reflog 视图 |

### 总菜单与主要子菜单

| 键 | 菜单 | 键 | 菜单 |
|----|------|----|------|
| `b` | Branch | `c` | Commit |
| `f` | Fetch | `F` | Pull |
| `P` | Push | `m` | Merge |
| `M` | Remote | `r` | Rebase |
| `t` | Tag | `z` | Stash |
| `X` | Reset | `i` | Ignore |
| `l` / `L` | Log / Log 参数 | `d` / `D` | Diff / Diff 参数 |
| `A` | Cherry-pick | `V` | Revert |
| `w` / `W` | Apply patches / Patch | `y` | Show refs |
| `I` / `C` | Init / Clone | `!` | Run |

子菜单参考 Magit transient 定义实现键位与参数开关。merge / rebase / cherry-pick / revert / am / bisect 菜单按仓库状态选择入口：操作进行中时显示 continue / skip / abort 等后缀，平时显示发起操作的后缀。例如：

- **Commit (`c`)**：`-a/-e/-v/-n/-R` 参数；`c` 提交、`e` Extend（--amend --no-edit）、`a` Amend、`w` Reword、`f` Fixup、`s` Squash、`F` Instant fixup、`S` Instant squash；level 0（默认不显示，可用 `transient-level magit-commit d 1` 打开）：`d` Reshelve（修改 HEAD 的提交者日期，自己是作者且未带 `C-u` 时同时修改作者日期；初始值为当前时间，`M-p` 取原日期）、`R` Reword past（同 `r w`）；level 6 的"Spread across commits"：`x` 用 `git-autofixup` 把改动分散为 fixup 提交（从 log 中选基准提交，初始停在与 upstream 的 merge-base；带 `C-u` 打开参数菜单 `-c` context、`-s` strict、`-v`(-vv，默认开)）、`X` 为每个已更新的子模块创建 `fixup! <最近修改它的提交>` 提交
- **Stash (`z`)**：与 Magit 一样不用 `git stash push`，而是用 `commit-tree` 构造 stash 提交后写入 `refs/stash`，再移除对应改动：`z` both、`i` index（只存暂存区改动，反向应用到暂存区和工作区，未暂存改动保留）、`w` worktree（暂存区保留，stash 的第一个父提交为 "pre-stash index"）、`x` keeping index；消息为空时使用 `On <分支>: <hash> <subject>`；`-u/-a` 或菜单内 `C-u`/`C-u C-u` 包含未跟踪/忽略文件；merge 冲突中 stash both 会先确认；没有对应改动时提示 `No staged/unstaged/local changes to save`。`Z/I/W` snapshot（消息 `WIP on ...`，不移除改动）、`r` 提交到 wip ref；`P` push（level 5，子菜单 `git stash push`：`--` 限定文件（候选为已修改文件，逗号分隔）、`-u/-a/-k/-K`）；`a/p` apply/pop：先 `stash apply --index`，失败再不带 `--index`，仍失败时若相关文件有未暂存改动则询问 `[s]` 暂存后 `git apply --3way` / `[r]` `--reject` / `[c]` 取消；`k` drop；`l/v` list/show；`b` 新分支（`git stash branch`）、`B` 在当前分支处建分支并应用 stash、`f` 生成 `0001-<subject>.patch`
- **Reset (`X`)**：`b` 重置分支、`f` 从某个版本检出文件（候选含目录，如 `dir/`）；`m/s/h/k/i/w` mixed/soft/hard/keep/index/worktree
- **Tag (`t`)**：`t` 创建（名称可补全已有 tag，`C-u` 时加 `--annotate`，消息由编辑器输入）、`r` release（按 `v1.2`/`version-1.2`/`release/1.2` 等格式找出版本最高的 release tag：HEAD 的提交信息为 `Release version V` 时直接用 V，否则以上一个 tag 为初始输入读取；带 `-a/-s/-u` 时消息取上一个 release 的消息并替换版本号；第一个 release 加 `--edit`，完成后打开 refs 缓冲区）、`k` 删除（多选时逐个列出确认）、`p` prune（读取远端，分别确认删除只存在于本地的 tag 和只存在于远端的 tag）
- **Worktree (`Z` / `%`)**：`b` 在新 worktree 中 checkout（候选排除已在其他 worktree 检出的分支）、`c` 新建分支并放到新 worktree（同 `b c` 先读起点再读名称）；目录按 `magit-read-worktree-directory-function` 读取，默认 `-sibling`：初始值为当前 worktree 旁边的 `<名称前缀>_<分支，/ 换成 ->`，也可设为 `magit-read-worktree-directory`、`-nested`（当前 worktree 内）、`-offsite`（`magit-read-worktree-offsite-directory`，默认 `~/.local/share/wtrees/`）；创建后切换到新 worktree 的 status。`m` 移动（主 worktree 不可移动；移动当前 worktree 时 status 跟随过去）、`k` 删除（主 worktree、加锁的 worktree 拒绝；有未提交改动时按 danger 确认，可移到回收站）、`g` 打开另一个 worktree 的 status
- **Ignore (`i`)**：`t` 写入顶层 `.gitignore`、`s` 写入子目录的 `.gitignore`（先读取目录）、`p` 写入 `$GIT_DIR/info/exclude`、`g` 写入 `core.excludesFile`（描述中显示该文件）；规则候选为未跟踪文件/目录（`/path`）及其扩展名（`*.ext`、`dir/*.ext`），默认取当前文件；写入 `.gitignore` 后自动暂存。level 7：`w/W` skip-worktree、`u/U` assume-unchanged（只有一个候选时不询问）
- **Fetch (`f`)**：`-p` prune、`-t` 全部 tag、`-F` force（`-u --unshallow` 在 level 7）；`p` 从 pushRemote（未设置或无效时先读取并设置 `branch.<b>.pushRemote`）、`u` 从"当前"远端（upstream 的远端，否则唯一的远端，否则主远端；都没有时不显示）、`e` 其他远端、`a` 全部远端（`fetch --all`）、`o` 某远端或 url 的一个分支、`r` 显式 refspec（候选来自 `ls-remote`）、`m` submodules（`fetch --recurse-submodules --verbose --jobs=4`；带 `C-u` 时打开它的菜单）、`C` 配置。`p`/`u` 的描述随仓库状态变化，例如 `pushRemote, setting that`
- **Pull (`F`)**：`-f` ff-only、`-r` rebase（选项：true/merges/interactive/false）、`-F` force（`-A` autostash 在 level 7）；`p` 从 pushRemote、`u` 从 upstream（未设置时先读取并设置）、`e` 某个远程分支、`U`（level 7）把作为 upstream 的本地分支更新到它自己的 upstream、`r` `branch.<b>.rebase`（git 变量后缀，见下）、`C` 配置；detached HEAD 时不显示 `p/u/r`
- 与 Transient 的 `:incompatible` 一致，打开某个参数时会清除与它互斥的参数：pull 的 `--ff-only` 与 `--rebase=true|merges|interactive`、merge 的 `--ff-only` 与 `--no-ff`、cherry-pick 的 `--ff` 与 `-x`
- **Push (`P`)**：`-f` force-with-lease、`-F` force、`-h` no-verify、`-n` dry-run、`-u` set-upstream（level 5）、`-T` 全部 tag、`-t` follow-tags、`-o` push-options（level 5，可多值，用逗号分隔；候选来自 `magit-push-options`，默认 `skip-ci ci.skip`）；`p` 推到 pushRemote（未设置或无效时先读取并设置；`set-and-push` 不在 `magit-no-confirm` 中时还要确认）、`u` 推到 upstream（未设置时读取任意 `REMOTE/BRANCH`，以 `--set-upstream` 推送）、`e` 把当前分支推到某个 `REMOTE/BRANCH`、`o` 推送其他分支或提交、`r` 显式 refspec（逗号分隔多个）、`m` matching、`T` 推一个 tag、`t` 推全部 tag、`n` 推一个 notes ref（level 6）、`C` 配置；`p/u` 的描述随状态变化（如 `origin/topic, creating it`），detached HEAD 时不显示 `p/u/e`
- **Merge (`m`)**：`m` merge、`e` 编辑消息后合并（`--no-ff`）、`n` 不提交、`a` absorb（若 push 目标存在先 `--force-with-lease` 推送，再 `merge --no-edit`，然后删除分支；对主分支会先确认）、`p` 预览（`git merge-tree`，不改动工作区）、`s` squash、`d` dissolve（切到目标分支并 absorb 原分支）；`-X` strategy-option、`-b`/`-w` 忽略空白、`-A` diff 算法（`-Xdiff-algorithm=`）在 level 5；合并进行中：`m` 提交合并、`a` 放弃
- **Rebase (`r`)**：`-r` 为选项 `--rebase-merges=`（no-rebase-cousins / rebase-cousins）；level 7 的 `-s` strategy、`-X` strategy-option、`=X` diff 算法、`-f` force-rebase、`-x` exec；`p` onto pushRemote（同 pull 的规则）、`u` onto upstream（未设置时读取并设置）、`e` elsewhere（任意分支或提交）、`i` interactive、`s` subset（`rebase --onto <新基> <起点>^`）、`m` 修改某个提交（todo 中改为 `edit`）、`w` reword、`k` 删除某个提交、`f` autosquash、`t` 修改日期（level 6，`magit-reshelve-since`：从 log 中选一个提交，读取它的新日期（初始值使各提交到现在每个相隔一分钟，历史中有"父提交之后一分钟"），之后的提交依次晚一分钟，用 `git filter-branch` 改写作者与提交者日期；`magit-reshelve-since-committer-only` 为 `t` 时只改提交者日期；使用 rebase 菜单的 `--gpg-sign=` 签名；detached HEAD 时拒绝，已有 `refs/original` 备份时先确认）；会改写已发布提交（`origin/master`）或范围内含 merge 时，按当前实现先确认；rebase 进行中：`r` continue、`s` skip、`e` 编辑 todo（`rebase --edit-todo`）、`a` abort
- **Cherry-pick (`A`)**：`h` harvest（把其他分支上的提交搬到当前分支，并从原分支移除）、`m` squash 合并、`d` donate（把当前分支上的提交搬到其他分支）、`n` spinout / `s` spinoff（用这些提交新建分支；spinoff 会切换过去）。被移走的提交是源分支 tip 时用 `update-ref` 回退源分支，否则用 `rebase -i` 删掉；进行中时 `A` continue、`s` skip、`a` abort
- 签名参数与 Magit 一致：commit、merge、rebase、cherry-pick、revert、am 都有 `-S --gpg-sign=`（level 5，描述随 `gpg.format` 变为 `Sign using ssh` 等）和 `+s --signoff`（level 6）；commit 另有 level 7 的 `-D --date=`。`-S` 与 tag 的 `-u --local-user=` 按 magit-read-gpg-signing-key 读取：`gpg.format` 不是 openpgp 时取空值；只有一个可签名的 secret key 或设置了 `magit-openpgp-default-signing-key` 时直接使用（带 `C-u` 时仍会询问）
- **Revert (`V`)**：进行中时 `V` continue、`s` skip、`a` abort
- **Apply patches (`w`)**：参数 `-3`（默认开启）、`-R`、`-p`（可为空）、`-c`、`-k`、`-b`、`-d`、`-t`、`-S`、`+s`；`m` 应用 mbox / maildir（`Apply mbox or Maildir: `）、`w` 应用 patch 文件（选中多个文件时直接使用，否则读取 `Apply patch (光标处文件): `）、`a` 打开 Apply plain patch 菜单。`git am` 进行中：`w` 继续（有未暂存改动时拒绝）、`s` 跳过、`a` 中止
- **Patch (`W`)**：`c` Create patches、`w` Apply patches、`a` Apply plain patch、`s` 保存 diff 为 patch（`Write patch file: `，去掉 `--stat`，带 `C-u` 时保留 buffer 的所有参数）、`r` request pull（读取 `Remote`、`Start`（默认上游分支）、`End`，显示 `git request-pull` 的输出）
  - **Create patches**：参数键位与 Magit 相同，以 `C-m`（即 RET）开头：level 6 的 `C-m C-r` in-reply-to、`C-m s` thread（`[d]eep` / `[s]hallow`）、`C-m C-f` / `C-m C-t` / `C-m C-c` from / to / cc（从 HEAD 的作者中补全）；`C-m b` base（默认 `auto`）、`C-m v` reroll count、level 5 的 `C-m d i` interdiff、`C-m d r` range-diff、`C-m p` subject prefix、`C-m r` RFC（与 subject prefix 互斥）、`C-m l` cover letter、level 5 的 `C-m D` cover-from-description、`C-m n` notes、`C-m o` 输出目录（必须已存在）；diff 参数 `-U`、`-M`、`-C`、`-A`、`--`（从已跟踪文件中选）及 level 7 的 `-b`、`-w`。`c` 创建：选中多个提交时使用 `最后一个^..第一个`，否则读取 `Create patches for range or commit: `，单个提交只生成该提交的 patch（`C^..C`）。带 `--cover-letter` 时创建后用编辑器（`git var GIT_EDITOR`）打开 `[v<N>-]0000-cover-letter.patch`
  - **Apply plain patch**：参数 `-i` --index、`-c` --cached、`-3` --3way；`a` 读取 `Apply patch: `（初始为光标处的文件）后执行 `git apply`
- **Notes (`T`)**：`c` / `d` 设置本仓库的 `core.notesRef` / `notes.displayRef`，`C` / `D` 设置全局的（菜单保持打开，描述显示当前值）；`core.notesRef` 未设置时从已有 notes ref 中读取（初始值为当前 `core.notesRef`，结果补全为 `refs/notes/...`），已设置时取消；`notes.displayRef` 用逗号分隔多个，当前值为初始输入。参数 `-n` dry run（prune）、`-r` 操作的 notes ref（edit / remove）、`-s` merge strategy；`T` 编辑（读取提交，默认为光标处的分支、提交或 stash，否则当前分支；用编辑器编辑 `git notes edit`）、`r` 删除、`m` 合并（`Merge reference`）、`p` prune（带 `-n` 时显示将删除的内容）。合并有冲突（`NOTES_MERGE_WORKTREE` 中有文件）时，菜单改为 `c` Commit merge、`a` Abort merge。与 Magit 一样，`-s` 不会传给 `git notes merge`
- **Bisect (`B`)**：未开始时：参数 `-n` --no-checkout、`-p` --first-parent（Bisect Rest 也只沿第一父提交），level 6 的 `=o` --term-old=、`=n` --term-new=；`B` 开始（region 选中两个提交或分支且一个是另一个的祖先时直接使用，否则读取 `Start bisect with bad revision` 和 `Good revision`，提示中的 bad/Good 随自定义术语变化；两者没有 merge-base 或有未提交改动时拒绝）、`s` 开始并运行脚本（`Bisect shell command: `，用 `$SHELL -c` 执行）。进行中：`B` bad、`g` good（按 `BISECT_TERMS` 中的术语标记 HEAD）、level 6 的 `m` 用 new/old 术语标记（`[n]ew` / `[o]ld`）、`k` skip、`r` reset（确认后删除保存的输出）、`s` 运行脚本。每步 `git bisect` 的输出保存在 `BISECT_CMD_OUTPUT`，显示在 status 的 bisect 输出区块中
- **Submodule (`o`)**：参数 `-f/-r/-N/-C/-R/-M/-U`，每个命令只取自己支持的参数（描述中方括号内的部分）。`a` 添加：依次读取远程 url、路径（初始值为 url 的最后一段，去掉 `.git`）和名称（默认为 `.gitmodules` 中该路径对应的名称，否则为路径），`.git/modules/<名称>` 已存在而路径不存在时可选 `u` 复用、`r` 改名（加 `-%F-%T` 后缀）、`t` 删除，添加后执行 `absorbgitdirs`。`r` 注册（`git submodule init`）、`p` populate（`update --init`，只列出没有工作区的模块）、`u` 更新、`s` 同步（只列出已 populate 的模块）、`d` unpopulate（`deinit`）都读取一个模块；带 `C-u` 时作用于所有符合条件的模块，并且总是先确认。`k` 删除：有未提交改动的模块会被跳过，带 `-f` 时确认后先在模块内 stash 再删除；带 `C-u` 或设置 `magit-submodule-remove-trash-gitdirs t` 时，确认后同时删除 `.git/modules/<名称>`。`l` 以表格列出模块（Path、Version、Branch、B<P、B<U、B>P、B>U、S、B 列），`f` fetch modules
- **Subtree (`O`)**：`i` Import、`e` Export 两个子菜单。Import：参数 `-P` prefix、`-m` message、`-s` squash；`a` 从仓库添加（读取远端或 url `From repository`，再从 `ls-remote` 的 ref 中读取 `Ref`）、`c` 添加某个提交、`m` 把提交合并进 subtree、`f` 从仓库 pull。Export：参数 `-P` prefix、`-a` annotate、`-b` branch、`-o` onto（读取版本，默认光标处的分支或提交，否则当前分支）、`-i` ignore joins、`-j` rejoin；`p` push 到仓库的某个 ref、`s` split（消息显示拆出的提交）。设置了 `-P` 时直接使用，否则以 `Add subtree: ` 等提示读取目录（相对于仓库根目录；绝对路径必须位于仓库内）。除 `--prefix=` 外的参数原样传给 `git subtree`
- **Branch (`b`)**：`b` checkout 分支/提交（远程分支、标签、提交会进入 detached HEAD）、`l` checkout 本地分支（选中远程分支时创建同名本地分支并跟踪它）、`c` 新建并 checkout（先读起点，光标处的远程分支为默认起点，再读名称）、`s` spin-off / `S` spin-out（未推送的提交移到新分支，当前分支重置到与 upstream 的分叉点；选中提交时重置到最旧选中提交的父提交）、`n` create、`m` rename、`k` delete；与 Magit 一样按 transient level 隐藏 `w/W` 新建 worktree（level 5）、`o` orphan 和 `-m --merge`（6）、`h/H` shelve/unshelve 和 `-r --recurse-submodules`（7），可在选项文件用 `transient-level magit-branch 7` 等打开；与 Magit 默认的 `magit-branch-direct-configure` 一致，菜单里直接提供 `d/u/r/p/R/P/B` 配置键；`C` 配置菜单：`d` description（打开编辑器，菜单退出）、`u` upstream（`branch.<b>.merge` 与 `.remote`，未设置时读取并设置，已设置时取消）、`r` `branch.<b>.rebase`（true/merges/interactive/false）、`p` `branch.<b>.pushRemote`（候选为各远端）、`R` `pull.rebase`、`P` `remote.pushDefault`、`B` 更新默认分支（fetch 主远端并 `remote set-head --auto`，把本地旧默认分支改名，同时修改跟踪它的分支的 upstream）、`a m` `branch.autoSetupMerge`（always/true/false）、`a r` `branch.autoSetupRebase`（always/local/remote/never）
- **git 变量后缀**（`b`、`F r`、`b C`、`M`、`M C` 中的变量键，对应 Magit 的 `magit--git-variable`）：描述显示变量名和当前值（未设置时为 `unset`）；可选值类显示为 `[a|(b)|c|fallback]`，当前值加括号，最后一项依次取 `global:值`、后备变量（如 `pull.rebase:true`、`remote.pushDefault:origin`）或 `default:值`，变量未设置时它加括号。按键后设置变量，菜单保持打开并刷新描述：可选值从未设置开始依次切换，最后一个之后变回未设置；带 `C-u` 时从候选中选择（选后备项即取消设置）；单值变量未设置时读取、已设置时取消；多值变量（`remote.<r>.fetch`、url）用逗号分隔，当前值作为初始输入。`b`、`F` 的作用域是当前分支，`M` 是当前远端；没有作用域（如 detached HEAD）时这些键不显示。从 `b` 进入 `b C`、从 `M` 进入 `M C`，或者带 `C-u` 进入时，会先读取要配置的分支/远端（`Configure branch` / `Configure remote`）
- **Remote (`M`)**：变量组 `u` `remote.<r>.url`、`U` `.fetch`、`s` `.pushurl`、`S` `.push`、`O` `.tagOpt`（--no-tags/--tags）、`h` `.followRemoteHEAD`（create/always/warn）；url 与 pushurl 通过 `git remote set-url [--push] --add/--delete` 增删，二者共用输入历史。`-f` 默认打开（添加后 fetch）；`a` 添加（url 初始值由 origin 的 url 把 owner 换成新远端名得到；按 `magit-remote-add-set-remote.pushDefault` 决定是否设置 `remote.pushDefault`，默认 `ask-if-unset`，可设为 `ask`、`nil` 或某个远端名）、`r` 改名、`k` 删除（改名/删除会同步修改 `remote.pushDefault` 与各分支的 `pushRemote`）、`C` 配置菜单、`p` prune 失效的远程分支、`P` 清理失效的 refspec（连同它们创建的远程分支；全部失效时可选择换成默认 refspec 或删除远端）、`z` unshallow（level 7；唯一的非通配 refspec 会询问是否换成标准 refspec）、`d u` 更新默认分支

- **Run (`!`)**：`!`/`p` 读取完整命令行（初始输入 `git `），`s`/`S` 读取任意 shell 命令；`!`/`s` 在仓库根目录、`p`/`S` 在启动目录用 `$SHELL -c` 执行，`GIT_PAGER=cat`，输出显示在文本视图并记入 `$` 进程日志；`k`/`a`/`b`/`g` 在后台启动图形程序。
- **Ediff (`E`)**：`E/u/i/w/c/r/z` 显示对应的文本 diff；`s` 用 `git add -p` 选择性暂存一个已跟踪文件；`m`/`M` 用 `git mergetool` 解决当前冲突；`t` 用 `git mergetool --gui`。`s` 在没有改动时、`m`/`M` 在没有未解决冲突时显示为括号内的 inapt 后缀（按下只提示 `Inapt command`）。
- **mergetool 参数**：在 `E`/`!` 菜单里先按 `C-u` 再按 `t`/`m` 时，打开 Magit 的 `magit-git-mergetool` 参数菜单：`-t` 覆盖 `--tool=`，`=t`/`=T` 设置全局 `merge.guitool`/`merge.tool`，`-r`/`-b`/`-k`/`-w` 切换全局 `mergetool.hideResolved`/`keepBackup`/`keepTemporaries`/`writeToTemp`，`SPC m`（先按空格键再按 `m`）执行 `git mergetool --gui ARGS -- FILE`。没有未解决冲突时提示 `There are no unresolved conflicts`。

菜单键按 Emacs `kbd` 的写法逐键输入：`-f` 是先按 `-` 再按 `f`，`a m` 是先按 `a` 再按 `m`（中间的空格只是记法分隔符）。mergetool 菜单的 `SPC m` 是例外：先按空格键，再按 `m`。只按了前缀时菜单会保持打开，等待后续按键；键序列没有定义时显示未绑定提示；`q` / `C-g` 退出菜单。

`r i`、`r w`、进行中的 `r e` 会暂时离开 TUI，打开 git 配置的编辑器（`GIT_SEQUENCE_EDITOR` / `GIT_EDITOR` / `core.editor`）编辑 todo 或提交消息，编辑器退出后回到 TUI。`r m`、`r k` 和 cherry 搬移按当前实现，用 perl 脚本改写 todo，不需要手动编辑，所以系统里要有 `perl`。

### 按键记法与区域选择

- `RET` 是 Enter，`TAB` 是 Tab；大写字母与小写字母不同。`C-n` 表示同时按 Ctrl+n，`M-n` 表示 Alt/Meta+n。
- `c c`、`b C a m` 均逐键输入，不输入空格；`P - f` 表示打开 Push 菜单后依次按 `-`、`f`，切换 `--force-with-lease`，再次输入可关闭。
- `C-u` 是前缀参数，可接数字或重复输入；例如 `C-u x` 使用 quick reset 的 hard 模式，风险高于默认 mixed 模式。前缀参数并非每条命令都支持。
- `C-SPC`（或 `C-@`）设置 mark，再移动光标选择 region；`C-g` 取消选区或当前交互。在展开的 diff 中可选择部分变更行，再执行适用的 `s`、`u`、`k` 等操作。
- `TAB` 切换当前 section；`Shift+TAB` 全局循环折叠层级，`1`–`4` 设置显示层级；`M-n`/`M-p` 移动到同级 section。
- 文本、log、diff 等视图有各自上下文键位；以上主要描述状态视图，操作前可查看菜单。

## 测试与开发

### 架构

```
src/
├── main.rs            # 入口：解析路径 → Git::discover → App
├── lib.rs             # 库入口（供测试引用）
├── git/
│   ├── exec.rs        # git 进程封装（全局参数/进程内查询短路/进程日志）
│   ├── libgit2.rs     # libgit2 只读后端（git2 crate：仓库发现/HEAD/status/refs）
│   ├── query.rs       # libgit2 只读查询层（刷新作用域内的白名单查询）
│   ├── ops.rs         # Magit 风格操作（stage/commit/branch/... + 附加功能）
│   └── status.rs      # magit-status 缓冲区模型（porcelain -z 解析）
└── ui/
    ├── keys.rs        # 按键解析（Emacs 标记 ↔ 事件）
    ├── transient.rs   # 全部 transient 菜单（数据化，可测试）
    ├── view.rs        # 状态缓冲区渲染（Head/Push/Untracked/Unstaged/Staged/Stashes）
    ├── paint.rs       # 样式行、按行类型的语义着色、整行绘制
    ├── theme.rs       # face 与深/浅配色、终端色深/背景/字形探测与降级
    ├── width.rs       # 终端显示宽度（CJK/emoji/组合字符/tab）
    ├── app.rs         # 事件循环 + 命令分发 + 提示交互 + 界面布局
    └── app/jobs.rs    # 后台刷新与后台 git 进程（结果经 channel 回到事件循环）
```

### libgit2 只读查询层

状态、refs、log 缓冲区每次刷新都会重复问同一批只读问题（HEAD 分支名、上游、配置值、提交标题、ahead/behind、ref 列表等）。这些查询由 `src/git/query.rs` 在进程内用 libgit2 回答，不必为每条启动一个 git 子进程；其余命令仍交给 git CLI。

- 刷新开始时，构建缓冲区的线程用 `query::Scope` 打开一次仓库（thread-local，libgit2 的 `Repository` 不是 `Sync`）。作用域只覆盖这一次刷新，嵌套作用域复用外层的仓库句柄。
- 只拦截参数形状完全匹配白名单的只读查询：`symbolic-ref --short HEAD`、`rev-parse`（`--verify`/`--short`/`--abbrev-ref`/`--quiet`/`-q`）、`config` 及 `config --get-all`/`--type=bool`、`rev-list --count` 与 `--left-right --count`、`log -1 --no-walk --format=...`（固定的几种格式）、`describe --long --tags`、`merge-base --is-ancestor`、`for-each-ref`（仅 `%(refname)`/`%(refname:short)`，且不带 `--contains`/`--points-at`/`--sort`/glob 等）、`remote`、`tag --list`、`stash list --format=%gd|%gs`。
- 其余情况一律回退 CLI：未知命令或参数、`diff`/`log --graph`/`blame`/`show` 等文本生成、带环境变量的调用（如 `GIT_INDEX_FILE`）、写操作、网络与凭据相关操作、编辑器和 hook，以及 libgit2 无法回答的查询（裸仓库、仓库打不开、解析不了的 rev 等）。查询失败时保留 CLI 的退出码语义，例如 `merge-base --is-ancestor` 返回非 0、`rev-parse --verify` 找不到对象时报失败。
- 已对齐的语义细节：`merge-base --is-ancestor A A` 成功，而 libgit2 不把提交视为自己的后代；`@{u}`/`@{upstream}` 不带分支前缀时取当前分支的上游；`%at` 用作者时间、`%ct` 用提交者时间；`for-each-ref` 按完整 refname 排序（即便输出短名）。`src/git/query.rs` 的单测逐项与真实 git CLI 比对输出和退出码。
- `git status --porcelain` 仍由 CLI 生成：XY 状态码、重命名/复制的源路径、unmerged 冲突分类与 `## ` 分支头都由 git 负责，用 libgit2 复刻容易与 Magit 的状态显示产生偏差。`Status::load()` 只读一次 `status --porcelain=v1 -z --branch`，同一个输出同时用于分支头与文件章节。
- 查询是刷新开始时的仓库快照，作用域内的只读查询都来自这一次快照；作用域外的调用仍读实时状态。实测在 ffmpeg 与本项目仓库中，启动时的 git 子进程数由 66–70 降到 16–18，状态页 `g` 由 33–35 降到 7–8（其中只剩一条 `status --porcelain`）。

### 测试

```bash
cargo test
```

测试位于 `tests/`，通过临时 Git 仓库、命令结果与应用按键断言覆盖行为，例如：

- `keymaps.rs`、`sequence.rs`、`prefix_arg.rs`：菜单定义、多键序列、Ctrl 与前缀参数。
- `git_ops.rs`、`branch_delete.rs`、`discard_files.rs`：Git 操作及仓库状态变化。
- `status_buffer.rs`、`diff_buffer.rs`：章节、hunk、region、暂存/取消暂存、apply/reverse 与 diff 参数作用域。
- `log_buffer.rs`、`log_select.rs`、`refs_buffer.rs`、`blame.rs`、`smerge.rs`：日志、选择、引用、blame 与冲突交互。
- `completion.rs`、`history.rs`、`transient_args.rs`、`transient_values.rs`、`options.rs`、`wip.rs`：补全、历史、参数持久化、配置及 WIP 行为。
- `run.rs`、`status_jump.rs`：run/ediff/mergetool 命令与进程日志、状态跳转与 imenu。

这里描述的是现有测试覆盖范围，不是对 Magit 全等的保证，也不代表所有平台已经验证。

## Magit 兼容范围与已知限制

xagit 参考 Magit 的键位、transient 菜单与 Git 参数语义；apply、多键参数、Ctrl 按键、`k` 丢弃、section/hunk/region 操作以及 bisect/worktree/submodule/notes/patch 均有实现，不应视为未支持占位。兼容不意味着 Emacs 界面和全部边界行为完全相同：

- 提交消息及 `m e` 的合并消息主要通过提示行输入；需要编辑 todo 或提交消息时使用外部编辑器，替代 Emacs 的 `git-rebase-mode`/with-editor 会话。编辑器退出后返回 TUI。
- `e`/`E` 用文本 diff 替代 Ediff，支持文件、暂存/未暂存、revision、commit、stash 等比较；`E s` 用 `git add -p` 替代 Ediff 的选择性暂存，`E m`/`E M`/`E t` 用 `git mergetool` 替代 Ediff 的冲突解决会话，效果取决于本机配置的 merge 工具，不等同于 Emacs 的三方 Ediff。
- 部分命令用统一文本视图替代 Magit 专用 buffer，不能据此推断完全没有结构化视图：项目已有专门的 status、diff、log、refs、blame 等模块。`!` 的 shell 命令与 `E t` 的 `git mergetool --gui` 同步捕获输出显示在文本视图，而不是 Magit 的异步 process buffer；`E m`/`E M` 与 `E s` 会把终端交给 merge 工具或 `git add -p`，`gitk`、`git gui` 等图形程序只在后台启动，不会嵌入 TUI。
- refs 候选、最近提交列表及提示行输入与 Magit 的历史/log 选择交互不同。例如部分 rebase 操作从最近 30 个提交中选择，cherry 搬移可输入 commit 或 `A..B` 区间。
- `D`/`L` 用于当前视图参数调整，按 `g` 刷新；现有测试断言 diff/log 参数修改只影响所显示的 buffer，新视图仍使用默认参数。旧的"运行内所有 diff/log 共享参数"说明不适用于当前实现。
- stash 采用原生 `git stash`，而 Magit 有自己的 stash 构造逻辑；现有测试不能证明所有场景的净效果一致，尚未通过全面的真实差分验证。
- 状态视图的文件状态仍来自 `git status --porcelain` 的 CLI 输出，而不是 libgit2 的状态 API；只有高频只读查询走进程内查询层，差异/日志文本、写操作与远程操作仍由 git CLI 执行。

## 贡献

欢迎在 [Issues](https://github.com/sigmaxue/xagit/issues) 提交问题或通过 Pull Request 改进代码和文档。报告问题时请提供复现步骤、Git/Rust 版本、终端环境以及相关错误输出；不要附带凭据或私有仓库敏感信息。

代码改动请补充对应的 `tests/` 回归用例并运行 `cargo test`；涉及键位或行为的变更请同步更新 README。提交前可用 `git diff --check` 检查空白问题。

## 致谢

感谢 [Magit](https://github.com/magit/magit) 提供的操作设计与键位参考，以及 Rust、[crossterm](https://github.com/crossterm-rs/crossterm)、[git2-rs](https://github.com/rust-lang/git2-rs) 和 libgit2 社区。xagit 是独立实现，不是 Magit 官方项目。

## 许可证

`Cargo.toml` 声明许可证为 `GPL-3.0-or-later`（GNU GPL v3.0 或更高版本）。当前仓库未包含独立的 `LICENSE`/`COPYING` 许可证全文文件；此处记录已声明的元数据，不补造许可证文件。
