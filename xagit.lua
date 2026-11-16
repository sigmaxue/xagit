-- Neovim (lazy.nvim)：插件位于仓库的 vim/ 子目录
return {
	"sigmaxue/xagit",
	build = "cargo build --release", -- 执行构建命令,
	init = function()
		local root = vim.fn.stdpath('data') .. '/lazy/xagit'
		vim.opt.rtp:append(root .. '/vim')
		vim.g.xagit_cmd = root .. '/target/release/xagit' -- 可省略
		vim.cmd('runtime! plugin/xagit.vim')
	end,
}
