# jieba-rs.nvim

Chinese word motions and text objects for Neovim, powered by
[jieba-rs](https://github.com/messense/jieba-rs).

`w` / `b` / `e` / `ge` / `iw` / `aw` and friends keep their exact native meaning
on ASCII text and additionally stop at Chinese **word** boundaries instead of
treating every hanzi as its own word.

This is a Neovim-only rewrite of [kkew3/jieba.vim](https://github.com/kkew3/jieba.vim)
(Apache-2.0). Upstream splits its integration layer across vimscript, a Python 3
backend for Vim 8/9, and a Rust engine reached through an `mlua` cdylib. Here the
**entire plugin is Rust**: the engine, the keymaps, the operator playback, the
preview and the configuration all live in one `cdylib`. The only Lua shipped is
`lua/jieba_rs/build.lua`, which runs `make install`. There is no vimscript and no
Python.

## Requirements

- **Neovim nightly (0.13-dev) or newer.** `src/nvimrs.rs` hand-rolls the Neovim
  C-API calls that nvim-oxi 0.6 gets wrong, and its `KeyDict_*` layouts are pinned
  to 0.13-dev's generated headers. On 0.12 and earlier those layouts differ, and
  the mismatch would be silent memory corruption rather than a clean error.
- A Rust toolchain on `PATH` to build the native module
- `make` (or, on Windows, `cargo` alone - the build helper falls back to it)
- Optional: [tpope/vim-repeat](https://github.com/tpope/vim-repeat) for `.` repeat

## Install

The compiled module (`lua/jieba_rs.so`) is not committed, so it has to be built
once after cloning or installing.

With Neovim's built-in `vim.pack`:

```lua
vim.pack.add { "git@github.com:litao91/jieba-rs.nvim.git" }
require("jieba_rs.build").build():pwait()   -- blocks until cargo finishes
require("jieba_rs").setup {
  filetypes = { "markdown", "typst", "rst" },
}
```

With [lazy.nvim](https://lazy.folke.io):

```lua
{
  "litao91/jieba-rs.nvim",
  build = function() require("jieba_rs.build").build():pwait() end,
  opts = { filetypes = { "markdown", "typst", "rst" } },
}
```

From a shell:

```sh
make install     # cargo build --release, then copy to lua/jieba_rs.so
```

A running Neovim keeps the module it already `dlopen`'d, so **restart after
building**.

## Configuration

The plugin is inert until `setup()` runs. Every key is optional.

```lua
require("jieba_rs").setup {
  lazy = true,                 -- defer loading the dictionary until Chinese appears
  user_dict = nil,             -- path to a jieba user dictionary (replaces the built-in one)
  mappings = true,             -- master switch for the default keymaps
  filetypes = {},              -- {} = map globally; non-empty = map buffer-locally on FileType
  preview_limits = 0,          -- 0 = preview the current line, N>0 = N steps, N<0 = unbounded
  experimental_opfunc = false, -- support arbitrary 'operatorfunc' (g@) via a visual selection
}
```

`filetypes` is the interesting one for a config that only wants Chinese word
motions in prose. With it set, `setup()` installs the default keymaps
**buffer-locally** from a `FileType` autocmd, so a `.rs` or `.py` buffer keeps
native `w`. With it empty the keymaps are global, matching upstream's
`g:jieba_vim_keymap`.

There are no `g:` variables. Configuration lives entirely in the `setup` table,
and all runtime state lives in Rust.

### `lazy`

With `lazy = true` (the default) the ~5 MB jieba dictionary is only parsed the
first time a line actually contains Chinese, so startup stays free. `lazy = false`
parses it during `setup()`. A `user_dict` is always loaded eagerly so that a bad
path is reported once, immediately, instead of from inside a keymap callback.

### `experimental_opfunc`

Operator-pending motions are implemented by *replaying* the operator over the
computed range. That has no side effects but cannot serve an arbitrary
`'operatorfunc'`: one that calls `getchar()` (mini.surround, a prompting `g@`)
would deadlock against the drained typeahead.

Setting `experimental_opfunc = true` makes `g@` take a different route - select the
range in Visual mode and let vim hand it to the operator - which supports any
`'operatorfunc'` at the cost of possibly clobbering `'<` / `'>`. Only `g@` is
affected; every other operator keeps using the playback path. See
[docs/design_choices/0003-omap-impl-tradeoff.md](docs/design_choices/0003-omap-impl-tradeoff.md),
carried over from jieba.vim.

## Mappings

`setup()` always defines these `<Plug>` maps, whether or not `mappings` is on:

| `<Plug>` | modes | native equivalent |
|---|---|---|
| `<Plug>(Jieba_w)` `<Plug>(Jieba_W)` | n, x, o | `w` `W` |
| `<Plug>(Jieba_b)` `<Plug>(Jieba_B)` | n, x, o | `b` `B` |
| `<Plug>(Jieba_e)` `<Plug>(Jieba_E)` | n, x, o | `e` `E` |
| `<Plug>(Jieba_ge)` `<Plug>(Jieba_gE)` | n, x, o | `ge` `gE` |
| `<Plug>(Jieba_C_Left)` `<Plug>(Jieba_S_Left)` | n, x, o, i | `<C-Left>` `<S-Left>` |
| `<Plug>(Jieba_C_Right)` `<Plug>(Jieba_S_Right)` | n, x, o, i | `<C-Right>` `<S-Right>` |
| `<Plug>(Jieba_iw)` `<Plug>(Jieba_iW)` | x, o | `iw` `iW` |
| `<Plug>(Jieba_aw)` `<Plug>(Jieba_aW)` | x, o | `aw` `aW` |
| `<Plug>(Jieba_C_w)` | i | `i_CTRL-W` |
| `<Plug>(Jieba_preview_w)` ... `<Plug>(Jieba_preview_gE)` | n | preview of that motion |
| `<Plug>(Jieba_preview_cancel)` | n | clear the preview |

So instead of the defaults:

```lua
require("jieba_rs").setup { mappings = false }
vim.keymap.set({ "n", "x", "o" }, "w", "<Plug>(Jieba_w)")
vim.keymap.set({ "x", "o" }, "iw", "<Plug>(Jieba_iw)")
vim.keymap.set("n", "<Leader>jw", "<Plug>(Jieba_preview_w)")
```

Counts (`4w`, `c2e`, `3daw`), registers (`"ayiw`), every characterwise operator
(`d`, `c`, `y`, `g~`, `gu`, `>` ...) and `.` repeat (with vim-repeat installed)
all work.

### Preview

`jieba_preview_w()` highlights where the next few `w`s would land, so you can see
the word boundaries before moving. One function per motion, plus a cancel:

```lua
local jieba = require("jieba_rs")
vim.keymap.set("n", "<Leader>cw", jieba.jieba_preview_w, { desc = "Preview jieba w" })
vim.keymap.set("n", "<Leader>ce", jieba.jieba_preview_e, { desc = "Preview jieba e" })
vim.keymap.set("n", "<Leader>cc", jieba.jieba_preview_cancel, { desc = "Cancel jieba preview" })
```

Available: `jieba_preview_w` `_W` `_e` `_E` `_b` `_B` `_ge` `_gE` and
`jieba_preview_cancel`. Prefer these over the equivalent
`<Plug>(Jieba_preview_*)` maps: a `<Plug>` used as a mapping rhs only resolves
when the mapping is **not** `noremap`, which is an easy footgun, whereas a Lua
function needs no such care. The `<Plug>` maps remain for configs that want them,
and `:JiebaPreviewCancel` does the same as `jieba_preview_cancel()`.

The highlights use the `JiebaPreview` group (linked to `IncSearch`) and are drawn
with extmarks, so *every* position is marked - upstream's `:match` loop only ever
showed the last one, since `:match` is one-slot-per-window. Text objects and the
arrow keys cannot be previewed.

## Commands

| command | effect |
|---|---|
| `:JiebaPreviewCancel` | clear the motion preview |
| `:JiebaOmapPlayback` `:JiebaOmapVisual` `:JiebaOmapAbort` `:JiebaImapExec` | internal plumbing; see below |

The internal commands exist because nvim holds `expr_map_lock` while an `<expr>`
mapping's callback runs, so the callback may not touch the buffer or the undo
history. Each one therefore computes and stashes its result, then returns
`<Esc><Cmd>...<CR>`; the `<Esc>` cancels the pending operator and the `<Cmd>`
does the work in ordinary normal mode. There is nothing to configure here.

## Layout

```
src/
  nvimrs.rs   native nvim C-API FFI for the parts of the 0.13 ABI that
              nvim-oxi 0.6 gets wrong (call_function, echo, autocmd / keymap /
              user-command callbacks, set_option_value, set_hl)
  state.rs    Opts, State, and the pending-action records the <expr> callbacks
              stash for their <Cmd> handlers
  jieba.rs    the segmenter (OnceLock, so lazy and eager share one concrete type)
  buffer.rs   BufferLike over nvim_buf_get_lines, with a per-motion snapshot
  pos.rs      getcurpos/getpos/setpos/cursor, UTF-8-safe column clamping
  model.rs    engine calls and their outputs
  nmap.rs     normal mode
  xmap.rs     visual mode
  omap.rs     operator-pending playback + vim-repeat
  opfunc.rs   the experimental g@ / 'operatorfunc' path
  imap.rs     insert-mode CTRL-W and the arrow keys
  preview.rs  motion preview on extmarks
  keymap.rs   <Plug> maps, default keymaps, commands, autocmds
  util.rs     the vimscript primitives the above are built on
jieba_vim_rs_core/   the motion + tokenizer engine (from jieba.vim, unmodified)
jieba_vim_rs_test/   deterministic segmenter used as the engine's test double
lua/jieba_rs/build.lua   the only Lua in the plugin
tests/integration.rs     differential harness against native vim
```

## Tests

```sh
cargo test --workspace              # engine unit tests + the differential harness
make integration                    # install first, then just the harness
```

`tests/integration.rs` is the compatibility guarantee. For ~375 ASCII scenarios
(buffer x cursor x keystrokes) it runs a **fresh `nvim --headless` process twice** -
once with the plugin's mappings, once vanilla - and compares buffer text, cursor,
selection, mode, register and `'[`/`']`. Fresh processes matter: a single-process
harness leaks typeahead and visual state between cases and reports plugin bugs
that are really harness contamination.

`'<`/`'>` are compared through `getpos('v')` while still in Visual mode, because
vim only writes those marks when visual mode *ends*; the plugin has to leave and
re-enter visual mode to apply a computed selection, so the raw marks would differ
for implementation reasons rather than behavioural ones.

`jieba_vim_rs_core/tests/golden_master.rs` replays JSONL fixtures captured from a
live Vim oracle. Upstream regenerates them in CI from its vimscript test suite;
this port does not ship them, so that binary runs zero trials.

## Credits

- [kkew3/jieba.vim](https://github.com/kkew3/jieba.vim) (Apache-2.0) - the engine
  in `jieba_vim_rs_core/`, the test double in `jieba_vim_rs_test/`, and the
  semantics of every mapping ported here.
- [nvim-matchup-rs](https://github.com/litao91/nvim-matchup-rs) (MIT) - `src/nvimrs.rs`
  derives from its FFI layer, including the compile-time `KeyDict_*` layout asserts
  that make it safe on nvim 0.13-dev.
- [jieba-rs](https://github.com/messense/jieba-rs) (MIT) - segmentation.
- [nvim-oxi](https://github.com/noib3/nvim-oxi) - the Neovim binding.

## License

Apache-2.0, see [LICENSE.txt](LICENSE.txt). A few files derived from Vim's own
runtime are additionally covered by [vim-LICENSE.txt](vim-LICENSE.txt).
