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
  local codes = vim.api.nvim_replace_termcodes(keys, true, true, true)
  pcall(vim.api.nvim_feedkeys, codes, "xt", false)
  vim.cmd("redraw")
end

-- The window layout, for cases that ask (`"layout": true`): Neovim's `winlayout()` with each
-- window's buffer, size, cursor, top line and columns of the top line scrolled off.
local function layout()
  local function node(n)
    if n[1] == "leaf" then
      local w = n[2]
      local pos = vim.api.nvim_win_get_cursor(w)
      local info = vim.fn.getwininfo(w)[1]
      return {
        "leaf",
        vim.fn.fnamemodify(vim.api.nvim_buf_get_name(vim.api.nvim_win_get_buf(w)), ":t"),
        vim.api.nvim_win_get_height(w),
        vim.api.nvim_win_get_width(w),
        { pos[1] - 1, pos[2] },
        info.topline - 1,
        w == vim.api.nvim_get_current_win(),
        vim.api.nvim_win_call(w, vim.fn.winsaveview).skipcol,
      }
    end
    local children = {}
    for _, c in ipairs(n[2]) do
      children[#children + 1] = node(c)
    end
    return { n[1], children }
  end
  return node(vim.fn.winlayout())
end

local out = {}
local root = vim.fn.tempname()
for i, case in ipairs(cases) do
  -- Each case gets its own directory holding `main.txt` and any extra files it names.
  local dir = root .. "/" .. i
  vim.fn.mkdir(dir, "p")
  local tmp = dir .. "/main.txt"
  local input = case_lines(case)
  vim.fn.writefile(input, tmp)
  for name, contents in pairs(case.files or {}) do
    vim.fn.writefile(vim.split(contents, "\n", { plain = true }), dir .. "/" .. name)
  end
  vim.cmd("set all&")
  vim.cmd("silent! only!")
  vim.cmd("silent! %bwipeout!")
  vim.cmd("cd " .. vim.fn.fnameescape(dir))
  -- Start each case like a fresh `:edit`, which puts line 1 in the jumplist.
  vim.cmd("clearjumps")
  vim.cmd("silent edit! main.txt")
  for _, r in ipairs({ '"', "a", "b", "q", "0", "1", "2", "3", "-" }) do
    vim.fn.setreg(r, "")
  end
  vim.fn.histdel(":")
  vim.fn.histdel("/")
  vim.fn.setreg("/", "")
  vim.cmd("nohlsearch")
  vim.cmd("silent! delmarks A-Z0-9")
  vim.api.nvim_win_set_cursor(0, { case.cur[1] + 1, case.cur[2] })
  vim.cmd("redraw")
  local chunks = type(case.keys) == "table" and case.keys or { case.keys }
  for _, chunk in ipairs(chunks) do
    feed(chunk)
  end
  feed("<Esc>")

  local pos = vim.api.nvim_win_get_cursor(0)
  local lay = case.layout and layout() or nil
  local bufname = case.layout and vim.fn.fnamemodify(vim.api.nvim_buf_get_name(0), ":t") or nil
  local text = table.concat(vim.api.nvim_buf_get_lines(0, 0, -1, false), "\n")
  -- Generated inputs are long; only record their text if the keys changed it.
  if (case.gen or case.layout) and text == table.concat(input, "\n") then
    text = nil
  end
  -- Registers the case asks about: contents and type (`v`, `V`, or `^V{width}`).
  local regs = nil
  if case.regs then
    regs = {}
    for _, name in ipairs(case.regs) do
      regs[name] = { vim.fn.getreg(name), vim.fn.getregtype(name) }
    end
  end
  out[#out + 1] = {
    id = case.id,
    text = text,
    cur = { pos[1] - 1, pos[2] },
    top = vim.fn.line("w0") - 1,
    regs = regs,
    layout = lay,
    buf = bufname,
  }
end
vim.fn.writefile({ vim.json.encode(out) }, arg[2])
