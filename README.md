# ngit — Magit-style git TUI in Rust

参考 [magit/magit](https://github.com/magit/magit) 实现的 git 命令行操作与快捷键操作，行为保持一致，Rust 实现，全部命令有测试验收。

## 构建

```bash
cargo build --release
./target/release/ngit          # 当前目录
./target/release/ngit /path    # 指定仓库
```

## 快捷键（与 Magit 一致）

### 全局按键（magit-mode-map）

| 键 | 命令 | 说明 |
|----|------|------|
| `h` / `?` | magit-dispatch | 弹出总菜单 |
| `g` | magit-refresh | 刷新状态缓冲区 |
| `q` / `Esc` | bury-buffer | 退出文本视图 / 退出程序 |
| `TAB` | toggle-section | 折叠/展开 section |
| `RET` | visit-thing | 查看光标处内容（diff / stash / 文件） |
| `n` / `p` / `C-n` / `C-p` / 方向键 | 移动 | 上下移动（Magit 没有 vi 式移动：`k` 是 discard，`j` 是跳转） |
| `j` | status-jump | 跳转到指定 section |
| `^` | section-up | 跳到所在 section 的标题行 |
| `s` / `u` | stage / unstage | 暂存 / 取消暂存光标处文件 |
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

### 总菜单（dispatch popup，与 magit.el:330-403 一致）

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

每个子菜单的键位与参数开关均按 Magit transient 定义实现。merge / rebase / cherry-pick / revert / am / bisect 菜单与 Magit 的 `:if` 谓词一致：操作进行中时显示 continue / skip / abort 等后缀，平时显示发起操作的后缀。例如：

- **Commit (`c`)**：`-a/-e/-v/-n/-R` 参数；`c` 提交、`e` Extend（--amend --no-edit）、`a` Amend、`w` Reword、`f` Fixup、`s` Squash、`F` Instant fixup、`S` Instant squash
- **Stash (`z`)**：`z/i/w/x` stash both/index/worktree/keep-index；`a/p/k` apply/pop/drop；`l/v` list/show；`Z/I/W` snapshot；`-u/-a` 包含未跟踪/忽略文件
- **Reset (`X`)**：`m/s/h/k/i/w` mixed/soft/hard/keep/index/worktree
- **Push (`P`)**：`-f` force-with-lease、`-F` force、`-h` no-verify、`-n` dry-run、`-u` set-upstream、`-T` 全部 tag、`-t` follow-tags；`p` 推到 pushRemote（`branch.<b>.pushRemote`，回退到 `remote.pushDefault`）、`u` upstream、`e` 其他、`o` 其他分支、`r` 显式 refspec、`m` matching、`T` 推一个 tag、`t` 推全部 tag、`C` 配置
- **Merge (`m`)**：`m` merge、`e` 编辑消息后合并（`--no-ff`）、`n` 不提交、`p` 预览（`git merge-tree`，不改动工作区）、`s` squash；合并进行中：`m` 提交合并、`a` 放弃
- **Rebase (`r`)**：`p` onto pushRemote、`u` onto upstream、`e` elsewhere、`f` autosquash；rebase 进行中：`r` continue、`s` skip、`a` abort
- **Cherry-pick (`A`) / Revert (`V`)**：进行中时 `A`/`V` continue、`s` skip、`a` abort
- **Bisect (`B`)**：平时 `B` 开始、`s` 开始并运行脚本；bisect 进行中 `B` bad、`g` good、`k` skip、`r` reset、`s` 运行脚本
- **Submodule (`o`)**：`a` 添加、`r` 注册（`git submodule init`，不删除任何东西）、`p` populate（`update --init`）、`u` 更新、`s` 同步、`d` unpopulate（`deinit`）、`k` 删除（带确认）、`l` 列表、`f` fetch modules
- **Branch (`b`)**：`b/l/c` checkout、`s` spin-off、`S` spin-out、`n` create、`m` rename、`k` delete；`C` 配置：`d` description、`u` upstream、`r` 循环切换 `branch.<b>.rebase`、`p` pushRemote、`R` 循环切换 `pull.rebase`、`P` `remote.pushDefault`

## 架构

```
src/
├── main.rs            # 入口：解析路径 → Git::discover → App
├── lib.rs             # 库入口（供测试引用）
├── git/
│   ├── exec.rs        # git 进程封装（rev-parse/status 解析/全局参数）
│   ├── libgit2.rs     # libgit2 只读后端（git2 crate：仓库发现/HEAD/status/refs）
│   ├── ops.rs         # 全部 Magit 风格操作（stage/commit/branch/... + 附加功能）
│   └── status.rs      # magit-status 缓冲区模型（porcelain -z 解析）
└── ui/
    ├── keys.rs        # 按键解析（Emacs 标记 ↔ 事件）
    ├── transient.rs   # 全部 transient 菜单（数据化，可测试）
    ├── view.rs        # 状态缓冲区渲染（Head/Push/Untracked/Unstaged/Staged/Stashes）
    └── app.rs         # 事件循环 + 命令分发 + 提示交互
```

行为一致性来源：每个操作的 git 参数与 Magit Elisp docstring 中记录的一致（如 `magit-unstage-1` 无初始提交时用 `git rm --cached`、否则 `git reset HEAD --`；`magit-reset-index` 用 `git reset <commit> -- .` 等）。

## 测试验收（132 项全部通过）

```bash
cargo test
```

- `tests/keymaps.rs`（28 项）：逐一比对 Magit lisp 源码中每个 transient 的键位与动作定义，含附加功能菜单断言、「无 `unsupported` 占位」全量校验、状态缓冲区直接按键断言、进行中菜单（`@progress`）的选择逻辑，以及通过 `App::press` 实际按键验证打开的菜单
- `tests/git_ops.rs`（104 项）：每个命令在临时仓库中执行后断言真实仓库状态，另含 4 项 libgit2 后端一致性、11 项附加功能行为测试、8 项 Magit 差异回归测试（ahead/behind、重命名、冲突、upstream、reverse、gitignore 子目录、工作目录命令）与 11 项按键语义回归测试（submodule 注册/populate/unpopulate、配置循环切换、pushRemote/tags/matching/refspecs 推送、merge 预览与编辑消息、合并中提交、sequencer continue/skip/abort、am 与 rebase 区分、bisect 状态、autosquash、进程日志、diff/log 参数）：
  - stage/unstage/discard × 9（含无初始提交场景）
  - commit 系列 × 9（create/extend/amend/reword/fixup/squash/instant-fixup/instant-squash/-a）
  - branch × 8（checkout/create/delete/rename/spinoff/spinout）
  - remote × 3、fetch/pull/push × 3、tag × 2
  - merge × 4（ff/no-commit/squash/abort 冲突恢复）
  - rebase × 2、reset × 6、revert × 2、cherry-pick × 2
  - stash × 10（both/index/worktree/keep-index/snapshot/pop/drop/clear/show/branch）
  - ignore/untrack/log/diff/show 等
  - 附加功能 × 11（bisect/worktree/submodule/notes/ediff 6 项，reflog/branch config/cherries/patches/subtree 5 项）

## 与 Magit 的差异说明

- 提交消息通过提示行输入（`-m`）而非弹出 Emacs 编辑器
- bisect / worktree / submodule / notes 已支持：`B` / `Z`（或 `%`）/ `o` / `T` 打开对应菜单，底层分别调用 `git bisect`、`git worktree`、`git submodule`、`git notes`
- ediff 没有 Emacs 会话：`e` / `E` 改为在文本视图显示对应 diff（光标处文件、暂存/未暂存、两个 rev 区间、单个 commit、stash）；Magit ediff 菜单中依赖 ediff 会话的 stage / resolve 未提供
- `D` / `L` 修改的参数在本次运行内对所有 diff / log 视图生效（Magit 为每个 buffer 单独保存）
- `m e` 的合并消息在提示行输入，而不是打开编辑器
- 需要编辑 todo 列表或专用 buffer 的 Magit 后缀未提供（按下无反应，不会触发别的命令）：merge `a` absorb / `d` dissolve，rebase `i/s/m/w/k` 与进行中的 `e`，cherry-pick `h/m/d/n/s`，am `m` maildir，branch configure `B` / `a m` / `a r`
- dispatch 与所有子菜单命令均已支持，无 `unsupported` 占位：`H`/`J` 输出当前行信息与状态缓冲区文本，`l r`/`l O`/`l H` 显示 reflog，`l f` 显示光标处文件的日志（ngit 附加），`O` subtree，`Y` cherries，`b C` 分支配置
- subtree / cherries / patches / 分支配置没有独立 Magit buffer，结果统一显示在文本视图
- stash 实现采用 `git stash` 原生命令（Magit 自建 stash commit），净效果一致
