# Changelog

All notable changes to this project are documented here. This project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0]

Initial release. A Neovim-only rewrite of
[kkew3/jieba.vim](https://github.com/kkew3/jieba.vim) 0.7.x.

### Changed

- **The whole integration layer moved from vimscript into Rust.** Upstream split
  itself three ways: vimscript for the Neovim/Vim integration, a Python 3 backend
  for Vim 8/9, and a Rust engine reached through an `mlua` lua51 cdylib plus a Lua
  shim. This port is a single Rust `cdylib` built on `nvim-oxi`: the keymaps, the
  operator playback, the visual-mode handling, insert-mode `CTRL-W`, the preview and
  the configuration are all Rust.
- **Vim 8/9 is no longer supported**, and with it the Python backend
  (`pythonx/`, `jieba_vim_rs_binding_py3`) and the `mlua` binding
  (`jieba_vim_rs_binding_lua51`) are gone.
- **Configuration moved from `g:` variables to a `setup()` table**
  (`g:jieba_vim_lazy` -> `lazy`, `g:jieba_vim_user_dict` -> `user_dict`,
  `g:jieba_vim_keymap` -> `mappings`, `g:jieba_vim_preview_limits` ->
  `preview_limits`, `g:jieba_vim_experimental_opfunc` -> `experimental_opfunc`).
  The plugin is inert until `setup()` runs, so there is no `plugin/` script and no
  `g:loaded_*` guard.
- **The loader is gone.** `require("jieba_rs")` succeeding *is* the check that the
  native module is present, so `g:jieba_vim_loaded_cdylib`,
  `g:jieba_vim_loaded_word_motion`, `g:jieba_vim_build_error` and
  `jieba_vim#install()` no longer exist. Building is `make install`, or
  `require("jieba_rs.build").build()` from Neovim.
- The motion preview is drawn with **extmarks** instead of `:match`.
- `src/nvimrs.rs` hand-rolls the Neovim C-API calls whose ABI nvim-oxi 0.6 gets
  wrong on 0.13-dev, with every `KeyDict_*` layout pinned by a compile-time
  `size_of!` / `offset_of!` assert.

### Added

- **`filetypes` option**: install the default keymaps buffer-locally from a
  `FileType` autocmd instead of globally, so the plugin can be restricted to prose
  filetypes without any config-side keymap code.
- **`tests/integration.rs`**, a differential harness that runs ~375 ASCII
  scenarios in a fresh `nvim --headless` process twice - once with the plugin's
  mappings, once vanilla - and compares buffer text, cursor, selection, mode,
  register and `'[` / `']`. This replaces upstream's Python metatest and its
  vimscript case DSL, neither of which survive the removal of the vimscript layer.
- `build.rs` passing `-Wl,-undefined,dynamic_lookup` on macOS, so the `nvim_*`
  symbols resolve at `dlopen` time rather than at link time.

### Fixed

- **The motion preview now highlights every position it reports.** Upstream looped
  `execute 'match JiebaPreview /.../'` once per position, but `:match` is
  one-slot-per-window, so all iterations but the last were no-ops.
- `:JiebaPreviewCancel` actually removes the highlights. Upstream only ran
  `hi clear JiebaPreview`, which made the leftover match invisible without
  deleting it.

### Removed

- The `model_funcname` hook that upstream's vimscript test suite used to inject an
  oracle model, and the `$JIEBA_TEST_CASE` branches it guarded.
- `docs/demo/` and `docs/benchmark/`, both of which drove the vimscript plugin.

[0.1.0]: https://github.com/litao91/jieba-rs.nvim/releases/tag/v0.1.0
