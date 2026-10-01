-- Dumps Neovim's default colorscheme for flux: every highlight group, for 'background' dark
-- and light, as `{"dark": {group: def}, "light": {...}}`, one group per line.
-- Usage: nvim --headless --clean -l colors.lua out.json

local GUI_ATTRS = { "bold", "italic", "underline", "undercurl", "strikethrough", "reverse" }

local function def(v)
  if v.link then
    return { link = v.link }
  end
  local d = { fg = v.fg, bg = v.bg, sp = v.sp, ctermfg = v.ctermfg, ctermbg = v.ctermbg }
  local gui, cterm = {}, {}
  for _, a in ipairs(GUI_ATTRS) do
    if v[a] then
      gui[#gui + 1] = a
    end
    if v.cterm and v.cterm[a] then
      cterm[#cterm + 1] = a
    end
  end
  if #gui > 0 then
    d.gui = gui
  end
  if #cterm > 0 then
    d.cterm = cterm
  end
  return d
end

local lines = { "{" }
for i, bg in ipairs({ "dark", "light" }) do
  vim.o.background = bg
  vim.cmd("colorscheme default")
  local groups = vim.api.nvim_get_hl(0, {})
  local names = vim.tbl_keys(groups)
  table.sort(names)
  lines[#lines + 1] = string.format('  "%s": {', bg)
  for j, name in ipairs(names) do
    local sep = j < #names and "," or ""
    lines[#lines + 1] = string.format("    %s: %s%s", vim.json.encode(name), vim.json.encode(def(groups[name])), sep)
  end
  lines[#lines + 1] = i == 1 and "  }," or "  }"
end
lines[#lines + 1] = "}"
vim.fn.writefile(lines, arg[1])
