-- Dumps Neovim's default colorscheme for flux: every highlight group, for 'background' dark
-- and light, as `{"dark": {group: def}, "light": {...}}`, one group per line.
-- Usage: nvim --headless --clean -l colors.lua out.json

local GUI_ATTRS = { "bold", "italic", "underline", "undercurl", "strikethrough", "reverse" }

-- One group as JSON, with its fields in a fixed order (a Lua table's key order isn't).
local function def(v)
  if v.link then
    return string.format('{"link":%s}', vim.json.encode(v.link))
  end
  local fields = {}
  for _, k in ipairs({ "fg", "bg", "sp", "ctermfg", "ctermbg" }) do
    if v[k] then
      fields[#fields + 1] = string.format('"%s":%d', k, v[k])
    end
  end
  local gui, cterm = {}, {}
  for _, a in ipairs(GUI_ATTRS) do
    if v[a] then
      gui[#gui + 1] = '"' .. a .. '"'
    end
    if v.cterm and v.cterm[a] then
      cterm[#cterm + 1] = '"' .. a .. '"'
    end
  end
  if #gui > 0 then
    fields[#fields + 1] = '"gui":[' .. table.concat(gui, ",") .. "]"
  end
  if #cterm > 0 then
    fields[#fields + 1] = '"cterm":[' .. table.concat(cterm, ",") .. "]"
  end
  return "{" .. table.concat(fields, ",") .. "}"
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
    lines[#lines + 1] = string.format("    %s: %s%s", vim.json.encode(name), def(groups[name]), sep)
  end
  lines[#lines + 1] = i == 1 and "  }," or "  }"
end
lines[#lines + 1] = "}"
vim.fn.writefile(lines, arg[1])
