-- Build and install the native module (lua/jieba_rs.so).
--
-- This is the only Lua in the plugin: everything else, including all of the
-- mapping logic, lives in Rust. The build itself is `make install`; this just
-- runs it without blocking the event loop and reports the outcome.
--
--   require("jieba_rs.build").build():pwait()   -- build, blocking (headless)
--   require("jieba_rs.build").build()           -- build, async + notifications
--   require("jieba_rs.build").library_available()

local M = {}

--- Absolute path of the plugin repository root.
--- This file is <root>/lua/jieba_rs/build.lua.
function M.get_repo_root()
  local src = debug.getinfo(1, "S").source:sub(2)
  return vim.fn.fnamemodify(src, ":p:h:h:h")
end

local function installed_path(root)
  local ext = vim.fn.has("win32") == 1 and "dll" or "so"
  return root .. "/lua/jieba_rs." .. ext
end

--- The cdylib cargo produces: `lib<name>.so` on Linux, `lib<name>.dylib` on
--- macOS and `<name>.dll` on Windows. Neovim's Lua C loader wants `.so` on every
--- unix, so the install step renames.
local function built_path(root)
  if vim.fn.has("win32") == 1 then return root .. "/target/release/jieba_rs_nvim.dll" end
  if vim.fn.has("mac") == 1 or vim.fn.has("macunix") == 1 then
    return root .. "/target/release/libjieba_rs_nvim.dylib"
  end
  return root .. "/target/release/libjieba_rs_nvim.so"
end

--- Whether the compiled native module is already installed.
---@return boolean
function M.library_available()
  return vim.uv.fs_stat(installed_path(M.get_repo_root())) ~= nil
end

local function notify(msg, level)
  -- Runs inside a vim.async task (fast-event context), where vim.notify is not
  -- allowed; vim.schedule'd work still runs while Task:pwait() blocks.
  vim.schedule(function()
    vim.notify("jieba_rs: " .. msg, level or vim.log.levels.INFO)
  end)
end

--- Build with `make install`, falling back to a bare `cargo build` plus a copy
--- where make is unavailable (Windows).
---
--- Returns an already-started |vim.async.Task|; block on it with `:pwait()`.
--- Cargo's own freshness check decides whether anything is rebuilt, so calling
--- this repeatedly is cheap and never churns an installed .so unnecessarily.
---@return vim.async.Task
function M.build()
  assert(vim.async and vim.async.run, 'require("jieba_rs.build").build() needs Neovim 0.12+')
  local root = M.get_repo_root()
  local cmd = vim.fn.executable("make") == 1
      and { "make", "install" }
    or { "cargo", "build", "--release" }

  return vim.async.run(function()
    notify("building the native module (this can take a few minutes)...")
    ---@diagnostic disable-next-line: redundant-parameter
    local res = vim.async.await(3, vim.system, cmd, { cwd = root, text = true })
    if res.code ~= 0 then
      notify(("build failed (exit %s)\n%s"):format(tostring(res.code), res.stderr or ""), vim.log.levels.ERROR)
      error("jieba_rs: build failed", 0)
    end
    if cmd[1] ~= "make" then
      vim.uv.fs_copyfile(built_path(root), installed_path(root))
    end
    notify("native module built; restart Neovim to load it")
    return true
  end)
end

return M
