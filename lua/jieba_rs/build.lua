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

--- Modification time in nanoseconds, or nil when the file does not exist.
local function mtime(path)
  local st = vim.uv.fs_stat(path)
  if not st then
    return nil
  end
  return st.mtime.sec * 1000000000 + (st.mtime.nsec or 0)
end

local function notify(msg, level)
  -- Runs inside a vim.async task (fast-event context), where vim.notify is not
  -- allowed; vim.schedule'd work still runs while Task:pwait() blocks.
  vim.schedule(function()
    vim.notify("jieba_rs: " .. msg, level or vim.log.levels.INFO)
  end)
end

--- Build with `make install`, falling back to a bare `cargo build` plus an atomic
--- install of the cdylib where make is unavailable (Windows).
---
--- Cargo's own freshness check decides whether anything is rebuilt, so this is
--- cheap to call on every start and stays silent when nothing changed. A
--- notification only appears when the installed module actually moved, and it says
--- which way out applies: a process that already loaded the module has to restart,
--- one that has not can require it straight away.
---
--- Returns an already-started |vim.async.Task|; block on it with `:pwait()`.
---@return vim.async.Task
function M.build()
  assert(vim.async and vim.async.run, 'require("jieba_rs.build").build() needs Neovim 0.12+')
  local root = M.get_repo_root()
  local installed = installed_path(root)
  local cmd = vim.fn.executable("make") == 1 and { "make", "install" } or { "cargo", "build", "--release" }
  local had_lib = M.library_available()
  local before = mtime(installed)
  if not had_lib then
    notify "building the native module (this can take a few minutes)..."
  end

  return vim.async.run(function()
    ---@diagnostic disable-next-line: redundant-parameter
    local res = vim.async.await(3, vim.system, cmd, { cwd = root, text = true })
    if res.code ~= 0 then
      notify(("build failed (exit %s)\n%s"):format(tostring(res.code), res.stderr or ""), vim.log.levels.ERROR)
      error("jieba_rs: build failed", 0)
    end
    if cmd[1] ~= "make" then
      -- The make path already skips a needless copy; mirror that here so the
      -- mtime comparison below still means "cargo relinked".
      local built = built_path(root)
      if not (vim.uv.fs_stat(installed) and mtime(built) <= mtime(installed)) then
        -- Install through a temp file and a rename, like `make install` does:
        -- copying over the destination in place rewrites the pages of a .so this
        -- Neovim may already have dlopen'd, while a rename swaps the directory
        -- entry and leaves the running inode mapped until restart.
        local tmp = installed .. ".tmp"
        vim.uv.fs_copyfile(built, tmp)
        -- A failed copy leaves no temp file behind, so the rename reports that too.
        local ok, err = vim.uv.fs_rename(tmp, installed)
        if not ok then
          vim.uv.fs_unlink(tmp)
          notify(("could not install %s: %s"):format(installed, tostring(err)), vim.log.levels.ERROR)
          error("jieba_rs: install failed", 0)
        end
      end
    end
    if not had_lib or mtime(installed) ~= before then
      local verb = had_lib and "rebuilt" or "built"
      if package.loaded.jieba_rs then
        -- This process already dlopen'd the previous module and keeps those pages
        -- until it exits, so only a restart picks the new one up.
        notify("native module " .. verb .. "; restart Neovim to load it")
      else
        -- Nothing is mapped yet, so the new module is reachable right away: this
        -- plugin ships lua/jieba_rs/, which keeps the module name in vim.loader's
        -- index of the directory, and the .so itself is stat'd on every lookup.
        notify("native module " .. verb .. "; require('jieba_rs') loads it now")
      end
    end
    return true
  end)
end

return M
