# xagit — Magit-style git TUI in Rust

xagit 是用 Rust 实现的终端 Git 界面，参考 [Magit](https://github.com/magit/magit) 的操作流程、快捷键和 transient 菜单。它结合 Git 命令行操作与 libgit2 只读后端，无需 Emacs 即可查看仓库状态并执行常用 Git 操作。

项目仓库：[sigmaxue/xagit](https://github.com/sigmaxue/xagit)。本项目不是 Magit 的完整替代，兼容范围与限制见下文。

## 目录

- [特性](#特性)
- [要求](#要求)
- [安装与构建](#安装与构建)
- [快速开始](#快速开始)
- [常用快捷键与多键解释](#常用快捷键与多键解释)
- [测试与开发](#测试与开发)
- [Magit 兼容范围与已知限制](#magit-兼容范围与已知限制)
- [贡献](#贡献)
- [致谢](#致谢)
- [许可证](#许可证)

## 特性

- 状态视图支持 section（章节）、文件、hunk（差异块）及 region（选中区域）操作，包括暂存、取消暂存、丢弃、应用与反向应用。可用操作取决于当前视图及变更类型。
- 提供分支、提交、远端、fetch/pull/push、merge/rebase、tag、stash、reset、cherry-pick 与 revert 菜单；进行中操作按仓库状态提供 continue/skip/abort 等入口。
- 支持 diff、log、refs、blame、补全、历史选择及 transient 参数；支持多键序列与 Ctrl/Meta 按键。
- 包含 bisect、worktree、submodule、subtree、notes、patch/am 与分支配置功能，以及进程日志视图。

## 要求

- Rust 工具链与 Cargo：项目使用 Rust 2021 edition，未声明最低 Rust 版本（MSRV）。建议使用当前稳定版。
- Git 命令行程序需位于 `PATH`；提交前需配置 Git 用户身份，远端操作需自行配置认证。
- 支持交互式终端的环境；本文不声明已验证的平台列表。
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
4. 按 `q` 返回上一视图；顶层状态视图中按 `q` 退出。

注意：`k` 是丢弃变更，不是向上移动。hard reset、强制推送、删除与历史改写等操作可能丢失数据，请检查目标、提示和参数，必要时先备份。

## 常用快捷键与多键解释

### 全局按键（magit-mode-map）

| 键 | 命令 | 说明 |
|----|------|------|
| `h` / `?` | magit-dispatch | 弹出总菜单 |
| `g` | magit-refresh | 刷新状态缓冲区 |
| `q` / `Esc` | bury-buffer | 退出文本视图 / 退出程序 |
| `TAB` | toggle-section | 折叠/展开 section |
| `RET` | visit-thing | 查看光标处内容（diff / stash / 文件） |
| `n` / `p`；`C-n` / `C-p` / 方向键 | 移动 | 前者按 section 移动，后者按行移动（Magit 没有 vi 式移动：`k` 是 discard，`j` 是跳转） |
| `j` | status-jump | 跳转到指定 section |
| `^` | section-up | 跳到所在 section 的标题行 |
| `s` / `u` | stage / unstage | 暂存 / 取消暂存光标处 section、文件、hunk 或选中 region |
| `S` / `U` | stage-modified / unstage-all | 全部暂存 / 全部取消暂存 |
| `k` | discard | 丢弃光标处变更（带确认） |
| `a` / `v` | apply / reverse | 把光标处变更（或 stash）应用到工作区 / 在工作区反向应用 |
| `K` | file-untrack | `git rm --cached` |
| `x` | reset-quickly | `git reset --mixed`（前缀参数为 --hard） |
| `:` / `Q` | git-command | 交互式 git 命令 |
| `!` | run 菜单 | 运行 git 命令 |
| `$` | process-buffer | 显示最近执行的 git 命令（含退出码与 stderr，失败行以 `!` 开头；查询类命令不记录） |
| `B` / `Z` / `%` / `o` / `O` / `T` | bisect / worktree / worktree / submodule / subtree / notes | 对应菜单 |
| `e` / `E` | ediff dwim / ediff 菜单 | 文本 diff 视图 |
| `D` / `L` | diff-refresh / log-refresh | 修改当前 diff / log 视图的参数，`g` 按新参数重新显示 |
| `V` | revert 菜单 | `V` 生成 revert 提交、`v` 只在工作区反向应用 |
| `H` / `J` / `Y` | section info / display buffer / cherries | 当前行信息、状态缓冲区文本、cherry 视图 |
| `w` / `W` | am / patch 菜单 | `git am` 菜单；patch 菜单（format-patch、保存 patch、request-pull） |
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

- **Commit (`c`)**：`-a/-e/-v/-n/-R` 参数；`c` 提交、`e` Extend（--amend --no-edit）、`a` Amend、`w` Reword、`f` Fixup、`s` Squash、`F` Instant fixup、`S` Instant squash
- **Stash (`z`)**：与 Magit 一样不用 `git stash push`，而是用 `commit-tree` 构造 stash 提交后写入 `refs/stash`，再移除对应改动：`z` both、`i` index（只存暂存区改动，反向应用到暂存区和工作区，未暂存改动保留）、`w` worktree（暂存区保留，stash 的第一个父提交为 "pre-stash index"）、`x` keeping index；消息为空时使用 `On <分支>: <hash> <subject>`；`-u/-a` 或菜单内 `C-u`/`C-u C-u` 包含未跟踪/忽略文件；merge 冲突中 stash both 会先确认；没有对应改动时提示 `No staged/unstaged/local changes to save`。`Z/I/W` snapshot（消息 `WIP on ...`，不移除改动）、`r` 提交到 wip ref；`P` push（level 5，子菜单 `git stash push`：`--` 限定文件（候选为已修改文件，逗号分隔）、`-u/-a/-k/-K`）；`a/p` apply/pop：先 `stash apply --index`，失败再不带 `--index`，仍失败时若相关文件有未暂存改动则询问 `[s]` 暂存后 `git apply --3way` / `[r]` `--reject` / `[c]` 取消；`k` drop；`l/v` list/show；`b` 新分支（`git stash branch`）、`B` 在当前分支处建分支并应用 stash、`f` 生成 `0001-<subject>.patch`
- **Reset (`X`)**：`b` 重置分支、`f` 从某个版本检出文件（候选含目录，如 `dir/`）；`m/s/h/k/i/w` mixed/soft/hard/keep/index/worktree
- **Tag (`t`)**：`t` 创建（名称可补全已有 tag，`C-u` 时加 `--annotate`，消息由编辑器输入）、`r` release（按 `v1.2`/`version-1.2`/`release/1.2` 等格式找出版本最高的 release tag：HEAD 的提交信息为 `Release version V` 时直接用 V，否则以上一个 tag 为初始输入读取；带 `-a/-s/-u` 时消息取上一个 release 的消息并替换版本号；第一个 release 加 `--edit`，完成后打开 refs 缓冲区）、`k` 删除（多选时逐个列出确认）、`p` prune（读取远端，分别确认删除只存在于本地的 tag 和只存在于远端的 tag）
- **Ignore (`i`)**：`t` 写入顶层 `.gitignore`、`s` 写入子目录的 `.gitignore`（先读取目录）、`p` 写入 `$GIT_DIR/info/exclude`、`g` 写入 `core.excludesFile`（描述中显示该文件）；规则候选为未跟踪文件/目录（`/path`）及其扩展名（`*.ext`、`dir/*.ext`），默认取当前文件；写入 `.gitignore` 后自动暂存。level 7：`w/W` skip-worktree、`u/U` assume-unchanged（只有一个候选时不询问）
- **Fetch (`f`)**：`-p` prune、`-t` 全部 tag、`-F` force（`-u --unshallow` 在 level 7）；`p` 从 pushRemote（未设置或无效时先读取并设置 `branch.<b>.pushRemote`）、`u` 从“当前”远端（upstream 的远端，否则唯一的远端，否则主远端；都没有时不显示）、`e` 其他远端、`a` 全部远端（`fetch --all`）、`o` 某远端或 url 的一个分支、`r` 显式 refspec（候选来自 `ls-remote`）、`m` submodules（`fetch --recurse-submodules --verbose --jobs=4`；带 `C-u` 时打开它的菜单）、`C` 配置。`p`/`u` 的描述随仓库状态变化，例如 `pushRemote, setting that`
- **Pull (`F`)**：`-f` ff-only、`-r` rebase（选项：true/merges/interactive/false）、`-F` force（`-A` autostash 在 level 7）；`p` 从 pushRemote、`u` 从 upstream（未设置时先读取并设置）、`e` 某个远程分支、`U`（level 7）把作为 upstream 的本地分支更新到它自己的 upstream、`r` `branch.<b>.rebase`（git 变量后缀，见下）、`C` 配置；detached HEAD 时不显示 `p/u/r`
- 与 Transient 的 `:incompatible` 一致，打开某个参数时会清除与它互斥的参数：pull 的 `--ff-only` 与 `--rebase=true|merges|interactive`、merge 的 `--ff-only` 与 `--no-ff`、cherry-pick 的 `--ff` 与 `-x`
- **Push (`P`)**：`-f` force-with-lease、`-F` force、`-h` no-verify、`-n` dry-run、`-u` set-upstream（level 5）、`-T` 全部 tag、`-t` follow-tags、`-o` push-options（level 5，可多值，用逗号分隔；候选来自 `magit-push-options`，默认 `skip-ci ci.skip`）；`p` 推到 pushRemote（未设置或无效时先读取并设置；`set-and-push` 不在 `magit-no-confirm` 中时还要确认）、`u` 推到 upstream（未设置时读取任意 `REMOTE/BRANCH`，以 `--set-upstream` 推送）、`e` 把当前分支推到某个 `REMOTE/BRANCH`、`o` 推送其他分支或提交、`r` 显式 refspec（逗号分隔多个）、`m` matching、`T` 推一个 tag、`t` 推全部 tag、`n` 推一个 notes ref（level 6）、`C` 配置；`p/u` 的描述随状态变化（如 `origin/topic, creating it`），detached HEAD 时不显示 `p/u/e`
- **Merge (`m`)**：`m` merge、`e` 编辑消息后合并（`--no-ff`）、`n` 不提交、`a` absorb（若 push 目标存在先 `--force-with-lease` 推送，再 `merge --no-edit`，然后删除分支；对主分支会先确认）、`p` 预览（`git merge-tree`，不改动工作区）、`s` squash、`d` dissolve（切到目标分支并 absorb 原分支）；`-X` strategy-option、`-b`/`-w` 忽略空白、`-A` diff 算法（`-Xdiff-algorithm=`）在 level 5；合并进行中：`m` 提交合并、`a` 放弃
- **Rebase (`r`)**：`-r` 为选项 `--rebase-merges=`（no-rebase-cousins / rebase-cousins）；level 7 的 `-s` strategy、`-X` strategy-option、`=X` diff 算法、`-f` force-rebase、`-x` exec；`p` onto pushRemote（同 pull 的规则）、`u` onto upstream（未设置时读取并设置）、`e` elsewhere（任意分支或提交）、`i` interactive、`s` subset（`rebase --onto <新基> <起点>^`）、`m` 修改某个提交（todo 中改为 `edit`）、`w` reword、`k` 删除某个提交、`f` autosquash、`t` 修改日期（level 6，`magit-reshelve-since`：从 log 中选一个提交，读取它的新日期（初始值使各提交到现在每个相隔一分钟，历史中有“父提交之后一分钟”），之后的提交依次晚一分钟，用 `git filter-branch` 改写作者与提交者日期；`magit-reshelve-since-committer-only` 为 `t` 时只改提交者日期；使用 rebase 菜单的 `--gpg-sign=` 签名；detached HEAD 时拒绝，已有 `refs/original` 备份时先确认）；会改写已发布提交（`origin/master`）或范围内含 merge 时，按当前实现先确认；rebase 进行中：`r` continue、`s` skip、`e` 编辑 todo（`rebase --edit-todo`）、`a` abort
- **Cherry-pick (`A`)**：`h` harvest（把其他分支上的提交搬到当前分支，并从原分支移除）、`m` squash 合并、`d` donate（把当前分支上的提交搬到其他分支）、`n` spinout / `s` spinoff（用这些提交新建分支；spinoff 会切换过去）。被移走的提交是源分支 tip 时用 `update-ref` 回退源分支，否则用 `rebase -i` 删掉；进行中时 `A` continue、`s` skip、`a` abort
- 签名参数与 Magit 一致：commit、merge、rebase、cherry-pick、revert、am 都有 `-S --gpg-sign=`（level 5，描述随 `gpg.format` 变为 `Sign using ssh` 等）和 `+s --signoff`（level 6）；commit 另有 level 7 的 `-D --date=`。`-S` 与 tag 的 `-u --local-user=` 按 magit-read-gpg-signing-key 读取：`gpg.format` 不是 openpgp 时取空值；只有一个可签名的 secret key 或设置了 `magit-openpgp-default-signing-key` 时直接使用（带 `C-u` 时仍会询问）
- **Revert (`V`)**：进行中时 `V` continue、`s` skip、`a` abort
- **Apply patches (`w`)**：`m` 应用 mbox / maildir（`git am <路径>`）、`w` 应用 patch 文件、`a` 应用普通 patch 到工作区
- **Bisect (`B`)**：平时 `B` 开始、`s` 开始并运行脚本；bisect 进行中 `B` bad、`g` good、`k` skip、`r` reset、`s` 运行脚本
- **Submodule (`o`)**：`a` 添加、`r` 注册（`git submodule init`，不删除任何东西）、`p` populate（`update --init`）、`u` 更新、`s` 同步、`d` unpopulate（`deinit`）、`k` 删除（带确认）、`l` 列表、`f` fetch modules
- **Branch (`b`)**：`b` checkout 分支/提交（远程分支、标签、提交会进入 detached HEAD）、`l` checkout 本地分支（选中远程分支时创建同名本地分支并跟踪它）、`c` 新建并 checkout（先读起点，光标处的远程分支为默认起点，再读名称）、`s` spin-off / `S` spin-out（未推送的提交移到新分支，当前分支重置到与 upstream 的分叉点；选中提交时重置到最旧选中提交的父提交）、`n` create、`m` rename、`k` delete；与 Magit 一样按 transient level 隐藏 `w/W` 新建 worktree（level 5）、`o` orphan 和 `-m --merge`（6）、`h/H` shelve/unshelve 和 `-r --recurse-submodules`（7），可在选项文件用 `transient-level magit-branch 7` 等打开；与 Magit 默认的 `magit-branch-direct-configure` 一致，菜单里直接提供 `d/u/r/p/R/P/B` 配置键；`C` 配置菜单：`d` description（打开编辑器，菜单退出）、`u` upstream（`branch.<b>.merge` 与 `.remote`，未设置时读取并设置，已设置时取消）、`r` `branch.<b>.rebase`（true/merges/interactive/false）、`p` `branch.<b>.pushRemote`（候选为各远端）、`R` `pull.rebase`、`P` `remote.pushDefault`、`B` 更新默认分支（fetch 主远端并 `remote set-head --auto`，把本地旧默认分支改名，同时修改跟踪它的分支的 upstream）、`a m` `branch.autoSetupMerge`（always/true/false）、`a r` `branch.autoSetupRebase`（always/local/remote/never）
- **git 变量后缀**（`b`、`F r`、`b C`、`M`、`M C` 中的变量键，对应 Magit 的 `magit--git-variable`）：描述显示变量名和当前值（未设置时为 `unset`）；可选值类显示为 `[a|(b)|c|fallback]`，当前值加括号，最后一项依次取 `global:值`、后备变量（如 `pull.rebase:true`、`remote.pushDefault:origin`）或 `default:值`，变量未设置时它加括号。按键后设置变量，菜单保持打开并刷新描述：可选值从未设置开始依次切换，最后一个之后变回未设置；带 `C-u` 时从候选中选择（选后备项即取消设置）；单值变量未设置时读取、已设置时取消；多值变量（`remote.<r>.fetch`、url）用逗号分隔，当前值作为初始输入。`b`、`F` 的作用域是当前分支，`M` 是当前远端；没有作用域（如 detached HEAD）时这些键不显示。从 `b` 进入 `b C`、从 `M` 进入 `M C`，或者带 `C-u` 进入时，会先读取要配置的分支/远端（`Configure branch` / `Configure remote`）
- **Remote (`M`)**：变量组 `u` `remote.<r>.url`、`U` `.fetch`、`s` `.pushurl`、`S` `.push`、`O` `.tagOpt`（--no-tags/--tags）、`h` `.followRemoteHEAD`（create/always/warn）；url 与 pushurl 通过 `git remote set-url [--push] --add/--delete` 增删，二者共用输入历史。`-f` 默认打开（添加后 fetch）；`a` 添加（url 初始值由 origin 的 url 把 owner 换成新远端名得到；按 `magit-remote-add-set-remote.pushDefault` 决定是否设置 `remote.pushDefault`，默认 `ask-if-unset`，可设为 `ask`、`nil` 或某个远端名）、`r` 改名、`k` 删除（改名/删除会同步修改 `remote.pushDefault` 与各分支的 `pushRemote`）、`C` 配置菜单、`p` prune 失效的远程分支、`P` 清理失效的 refspec（连同它们创建的远程分支；全部失效时可选择换成默认 refspec 或删除远端）、`z` unshallow（level 7；唯一的非通配 refspec 会询问是否换成标准 refspec）、`d u` 更新默认分支

菜单键按 Emacs `kbd` 的写法逐键输入：`-f` 是先按 `-` 再按 `f`，`a m` 是先按 `a` 再按 `m`。只按了前缀时菜单会保持打开，等待后续按键；键序列没有定义时显示未绑定提示；`q` / `C-g` 退出菜单。

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
│   ├── exec.rs        # git 进程封装（rev-parse/status 解析/全局参数）
│   ├── libgit2.rs     # libgit2 只读后端（git2 crate：仓库发现/HEAD/status/refs）
│   ├── ops.rs         # Magit 风格操作（stage/commit/branch/... + 附加功能）
│   └── status.rs      # magit-status 缓冲区模型（porcelain -z 解析）
└── ui/
    ├── keys.rs        # 按键解析（Emacs 标记 ↔ 事件）
    ├── transient.rs   # 全部 transient 菜单（数据化，可测试）
    ├── view.rs        # 状态缓冲区渲染（Head/Push/Untracked/Unstaged/Staged/Stashes）
    └── app.rs         # 事件循环 + 命令分发 + 提示交互
```

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

这里描述的是现有测试覆盖范围，不是本次运行结果，也不保证与 Magit 全等。本次仅修改文档，未运行测试，不提供测试通过数量或覆盖率声明。

## Magit 兼容范围与已知限制

xagit 参考 Magit 的键位、transient 菜单与 Git 参数语义；apply、多键参数、Ctrl 按键、`k` 丢弃、section/hunk/region 操作以及 bisect/worktree/submodule/notes/patch 均有实现，不应视为未支持占位。兼容不意味着 Emacs 界面和全部边界行为完全相同：

- 提交消息及 `m e` 的合并消息主要通过提示行输入；需要编辑 todo 或提交消息时使用外部编辑器，替代 Emacs 的 `git-rebase-mode`/with-editor 会话。编辑器退出后返回 TUI。
- `e`/`E` 用文本 diff 替代 Ediff，支持文件、暂存/未暂存、revision、commit、stash 等比较；依赖 Ediff 会话的 stage/resolve 入口仍未提供。普通暂存和冲突相关功能不因此全部缺失。
- 部分命令用统一文本视图替代 Magit 专用 buffer，不能据此推断完全没有结构化视图：项目已有专门的 status、diff、log、refs、blame 等模块。
- refs 候选、最近提交列表及提示行输入与 Magit 的历史/log 选择交互不同。例如部分 rebase 操作从最近 30 个提交中选择，cherry 搬移可输入 commit 或 `A..B` 区间。
- `D`/`L` 用于当前视图参数调整，按 `g` 刷新；现有测试断言 diff/log 参数修改只影响所显示的 buffer，新视图仍使用默认参数。旧的“运行内所有 diff/log 共享参数”说明不适用于当前实现。
- stash 采用原生 `git stash`，而 Magit 有自己的 stash 构造逻辑；现有测试不能证明所有场景的净效果一致，尚未通过全面的真实差分验证。
- 测试中的键位与仓库状态断言只是兼容性证据，不等于与 Magit 全等，也不代表所有平台已经验证。

## 贡献

欢迎在 [Issues](https://github.com/sigmaxue/xagit/issues) 提交问题或通过 Pull Request 改进代码和文档。报告问题时请提供复现步骤、Git/Rust 版本、终端环境以及相关错误输出；不要附带凭据或私有仓库敏感信息。

代码改动请补充对应的 `tests/` 回归用例并运行 `cargo test`；涉及键位或行为的变更请同步更新 README。提交前可用 `git diff --check` 检查空白问题。

## 致谢

感谢 [Magit](https://github.com/magit/magit) 提供的操作设计与键位参考，以及 Rust、[crossterm](https://github.com/crossterm-rs/crossterm)、[git2-rs](https://github.com/rust-lang/git2-rs) 和 libgit2 社区。xagit 是独立实现，不是 Magit 官方项目。

## 许可证

`Cargo.toml` 声明许可证为 `GPL-3.0-or-later`（GNU GPL v3.0 或更高版本）。当前仓库未包含独立的 `LICENSE`/`COPYING` 许可证全文文件；此处记录已声明的元数据，不补造许可证文件。
