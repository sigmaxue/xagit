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
| `j` / `k` / 方向键 | 移动 | 上下移动 |
| `s` / `u` | stage / unstage | 暂存 / 取消暂存光标处文件 |
| `S` / `U` | stage-modified / unstage-all | 全部暂存 / 全部取消暂存 |
| `k` | discard | 丢弃光标处变更（带确认） |
| `K` | file-untrack | `git rm --cached` |
| `x` | reset-quickly | `git reset --mixed`（前缀参数为 --hard） |
| `:` / `Q` | git-command | 交互式 git 命令 |
| `$` / `!` | run 菜单 | 运行 git 命令 |

### 总菜单（dispatch popup，与 magit.el:330-403 一致）

| 键 | 菜单 | 键 | 菜单 |
|----|------|----|------|
| `b` | Branch | `c` | Commit |
| `f` | Fetch | `F` | Pull |
| `P` | Push | `m` | Merge |
| `M` | Remote | `r` | Rebase |
| `t` | Tag | `z` | Stash |
| `X` | Reset | `i` | Ignore |
| `l` | Log | `d` | Diff |
| `A` | Cherry-pick | `V` | Revert |
| `y` | Show refs | `I` / `C` | Init / Clone |

每个子菜单的键位与参数开关均按 Magit transient 定义实现，例如：

- **Commit (`c`)**：`-a/-e/-v/-n/-R` 参数；`c` 提交、`e` Extend（--amend --no-edit）、`a` Amend、`w` Reword、`f` Fixup、`s` Squash、`F` Instant fixup、`S` Instant squash
- **Stash (`z`)**：`z/i/w/x` stash both/index/worktree/keep-index；`a/p/k` apply/pop/drop；`l/v` list/show；`Z/I/W` snapshot；`-u/-a` 包含未跟踪/忽略文件
- **Reset (`X`)**：`m/s/h/k/i/w` mixed/soft/hard/keep/index/worktree
- **Push (`P`)**：`-f` force-with-lease、`-F` force、`-n` dry-run、`-u` set-upstream、`t` 推 tag
- **Branch (`b`)**：`b/l/c` checkout、`s` spin-off、`S` spin-out、`n` create、`m` rename、`k` delete

## 架构

```
src/
├── main.rs            # 入口：解析路径 → Git::discover → App
├── lib.rs             # 库入口（供测试引用）
├── git/
│   ├── exec.rs        # git 进程封装（rev-parse/status 解析/全局参数）
│   ├── ops.rs         # 全部 Magit 风格操作（stage/commit/branch/...）
│   └── status.rs      # magit-status 缓冲区模型（porcelain -z 解析）
└── ui/
    ├── keys.rs        # 按键解析（Emacs 标记 ↔ 事件）
    ├── transient.rs   # 全部 transient 菜单（数据化，可测试）
    ├── view.rs        # 状态缓冲区渲染（Head/Push/Untracked/Unstaged/Staged/Stashes）
    └── app.rs         # 事件循环 + 命令分发 + 提示交互
```

行为一致性来源：每个操作的 git 参数与 Magit Elisp docstring 中记录的一致（如 `magit-unstage-1` 无初始提交时用 `git rm --cached`、否则 `git reset HEAD --`；`magit-reset-index` 用 `git reset <commit> -- .` 等）。

## 测试验收（90 项全部通过）

```bash
cargo test
```

- `tests/keymaps.rs`（20 项）：逐一比对 Magit lisp 源码中每个 transient 的键位与动作定义
- `tests/git_ops.rs`（70 项）：每个命令在临时仓库中执行后断言真实仓库状态：
  - stage/unstage/discard × 9（含无初始提交场景）
  - commit 系列 × 9（create/extend/amend/reword/fixup/squash/instant-fixup/instant-squash/-a）
  - branch × 8（checkout/create/delete/rename/spinoff/spinout）
  - remote × 3、fetch/pull/push × 3、tag × 2
  - merge × 4（ff/no-commit/squash/abort 冲突恢复）
  - rebase × 2、reset × 6、revert × 2、cherry-pick × 2
  - stash × 10（both/index/worktree/keep-index/snapshot/pop/drop/clear/show/branch）
  - ignore/untrack/log/diff/show 等

## 与 Magit 的差异说明

- 提交消息通过提示行输入（`-m`）而非弹出 Emacs 编辑器
- bisect/worktree/submodule/ediff/notes 等 Magit 附加功能标记为 `unsupported`
- stash 实现采用 `git stash` 原生命令（Magit 自建 stash commit），净效果一致
