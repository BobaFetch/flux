-- Runs flux's oracle cases through Neovim and records the result of each.
-- Usage: nvim --headless --clean -l oracle.lua cases.json out.json
--
-- A case's `keys` is a string, or a list of chunks. Each chunk is typed as a unit and followed
-- by a redraw, like someone pausing between commands. Split cases at command boundaries when
-- redraws matter: Neovim finishes some scrolling (CTRL-D over wrapped lines) only when it
-- redraws, which it skips while more typed keys are waiting.
--
-- The window is Neovim's headless default, 80x24 with a statusline, so the text area is 80x22.
-- flux's oracle test uses the same size.

local cases = vim.json.decode(table.concat(vim.fn.readfile(arg[1]), "\n"))

local function case_lines(case)
  if not case.gen then
    return vim.split(case.text, "\n", { plain = true })
  end
  local lines = {}
  for i = 1, case.gen.lines do
    local line = tostring(i)
    if case.gen.long_every and i % case.gen.long_every == 0 then
      line = line .. string.rep("x", case.gen.long_width)
    end
    lines[i] = line
  end
  return lines
end

local function feed(keys)
  local codes = vim.api.nvim_replace_termcodes(keys, true, false, true)
  pcall(vim.api.nvim_feedkeys, codes, "xt", false)
  vim.cmd("redraw")
end

local out = {}
local tmp = vim.fn.tempname() .. ".txt"
for _, case in ipairs(cases) do
  local input = case_lines(case)
  vim.fn.writefile(input, tmp)
  vim.cmd("silent! %bwipeout!")
  vim.cmd("silent edit! " .. vim.fn.fnameescape(tmp))
  for _, r in ipairs({ '"', "a", "0", "1", "2", "-", "q" }) do
    vim.fn.setreg(r, "")
  end
  vim.api.nvim_win_set_cursor(0, { case.cur[1] + 1, case.cur[2] })
  vim.cmd("redraw")
  local chunks = type(case.keys) == "table" and case.keys or { case.keys }
  for _, chunk in ipairs(chunks) do
    feed(chunk)
  end
  feed("<Esc>")

  local pos = vim.api.nvim_win_get_cursor(0)
  local text = table.concat(vim.api.nvim_buf_get_lines(0, 0, -1, false), "\n")
  -- Generated inputs are long; only record their text if the keys changed it.
  if case.gen and text == table.concat(input, "\n") then
    text = nil
  end
  out[#out + 1] = {
    id = case.id,
    text = text,
    cur = { pos[1] - 1, pos[2] },
    top = vim.fn.line("w0") - 1,
  }
end
vim.fn.writefile({ vim.json.encode(out) }, arg[2])
