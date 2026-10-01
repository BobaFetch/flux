-- A sample for comparing highlights with Neovim.
local M = {}

---@param name string
---@return string
function M.greet(name, ...)
  local count = select("#", ...)
  if name == nil or #name == 0 then
    return "hello, world"
  end
  for i = 1, count do
    print(i, true, false, 0x1F, 3.14)
  end
  local t = { a = 1, ["b"] = "two", [[long
string]], nested = { self = M } }
  return string.format("hello, %s\n", name) .. tostring(t.a)
end

local function helper(x) return x and not false end
M.helper = helper
goto done
::done::
return setmetatable(M, { __index = _G })
