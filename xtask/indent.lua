-- Indents flux's indent corpus with Neovim, for `cargo xtask indent gen|check`.
-- Usage: nvim --headless --clean -l indent.lua CORPUS_DIR OUT_DIR
--
-- For every file NAME in CORPUS_DIR, with the leading white space of each line removed:
--   NAME.reindent: the file after `gg=G`.
--   NAME.typed:    the lines typed into an empty NAME, `<CR>` between them, with comment
--                  leaders and auto-wrap off ('formatoptions' without `r` and `o`,
--                  'textwidth' 0) so only indenting is tested.

local corpus, out = arg[1], arg[2]

local function stripped(path)
  return vim.tbl_map(function(l)
    return (l:gsub("^%s+", ""))
  end, vim.fn.readfile(path))
end

local function fresh_dir()
  local dir = vim.fn.tempname()
  vim.fn.mkdir(dir, "p")
  vim.cmd("silent! %bwipeout!")
  vim.cmd("cd " .. vim.fn.fnameescape(dir))
  return dir
end

for _, name in ipairs(vim.fn.readdir(corpus)) do
  local path = corpus .. "/" .. name
  if vim.fn.isdirectory(path) == 0 and not name:match("^%.") then
    local lines = stripped(path)

    local dir = fresh_dir()
    vim.fn.writefile(lines, dir .. "/" .. name)
    vim.cmd("silent edit " .. vim.fn.fnameescape(name))
    vim.cmd("normal! gg=G")
    vim.fn.writefile(vim.api.nvim_buf_get_lines(0, 0, -1, false), out .. "/" .. name .. ".reindent")

    fresh_dir()
    vim.cmd("silent edit " .. vim.fn.fnameescape(name))
    local keys = ":setlocal fo-=r fo-=o tw=0\r" .. "i" .. table.concat(lines, "\r") .. "\27"
    vim.api.nvim_feedkeys(keys, "xt", false)
    vim.fn.writefile(vim.api.nvim_buf_get_lines(0, 0, -1, false), out .. "/" .. name .. ".typed")
  end
end
