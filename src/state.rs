// Copyright 2026 Kaiwen Wu. All Rights Reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License"); you may not
// use this file except in compliance with the License. You may obtain a copy
// of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS, WITHOUT
// WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. See the
// License for the specific language governing permissions and limitations
// under the License.

//! Plugin configuration and runtime state.
//!
//! Everything mutable the plugin owns lives here in Rust - there are no `g:`,
//! `b:` or `w:` variables and no Lua-side state.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use jieba_vim_rs_core::motion::WordMotion;
use jieba_vim_rs_core::token::Tokenizer;
use nvim_oxi::conversion::FromObject;
use nvim_oxi::{Array, Boolean, Dictionary, Object};

use crate::jieba::JiebaImpl;
use crate::pos::{Curpos5, Pos4};

pub type SharedState = Rc<State>;

/// Resolved `setup(opts)` configuration.
#[derive(Debug, Clone)]
pub struct Opts {
    /// Defer loading the jieba dictionary until a line actually contains
    /// Chinese. Upstream's `g:jieba_vim_lazy`.
    pub lazy: bool,
    /// Path to a user dictionary, which *replaces* the built-in one. Upstream's
    /// `g:jieba_vim_user_dict`.
    pub user_dict: Option<String>,
    /// Install the default keymaps (on top of the always-present
    /// `<Plug>(Jieba_*)` maps). Upstream's `g:jieba_vim_keymap`.
    pub mappings: bool,
    /// When non-empty, the default keymaps are installed **buffer-locally** from
    /// a `FileType` autocmd matching exactly these patterns, instead of
    /// globally. This is how a config restricts the plugin to, say, prose
    /// filetypes.
    pub filetypes: Vec<String>,
    /// Motion preview bound. Positive: preview that many steps; 0: preview the
    /// current line; negative: unbounded (99999). Upstream's
    /// `g:jieba_vim_preview_limits`.
    pub preview_limits: i64,
    /// Implement operator-pending maps through a visual selection so that any
    /// `'operatorfunc'` (including `g@`) works, at the cost of possibly
    /// clobbering `'<` / `'>`. Upstream's `g:jieba_vim_experimental_opfunc`.
    pub experimental_opfunc: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            lazy: true,
            user_dict: None,
            mappings: true,
            filetypes: Vec::new(),
            preview_limits: 0,
            experimental_opfunc: false,
        }
    }
}

fn obj_bool(o: &Object) -> Option<bool> {
    if let Ok(i) = i64::try_from(o.clone()) {
        return Some(i != 0);
    }
    Boolean::from_object(o.clone()).ok()
}

fn obj_i64(o: &Object) -> Option<i64> {
    i64::try_from(o.clone()).ok()
}

fn obj_string(o: &Object) -> Option<String> {
    String::from_object(o.clone()).ok()
}

impl Opts {
    /// Overlay a user's `setup(opts)` table on the defaults.
    pub fn from_opts(opts: Option<&Dictionary>) -> Self {
        let mut o = Self::default();
        let Some(opts) = opts else { return o };
        if let Some(v) = opts.get("lazy").and_then(obj_bool) {
            o.lazy = v;
        }
        if let Some(v) = opts.get("user_dict").and_then(obj_string) {
            // An empty string means "no user dictionary", as upstream did.
            o.user_dict = if v.is_empty() { None } else { Some(v) };
        }
        if let Some(v) = opts.get("mappings").and_then(obj_bool) {
            o.mappings = v;
        }
        if let Some(v) = opts.get("preview_limits").and_then(obj_i64) {
            o.preview_limits = v;
        }
        if let Some(v) = opts.get("experimental_opfunc").and_then(obj_bool) {
            o.experimental_opfunc = v;
        }
        if let Some(arr) = opts.get("filetypes").cloned()
            && let Ok(arr) = Array::from_object(arr)
        {
            o.filetypes = arr.iter().filter_map(obj_string).collect();
        }
        o
    }

    /// Effective preview bound, applying upstream's "negative means unbounded".
    pub fn preview_limit(&self) -> usize {
        if self.preview_limits < 0 {
            99999
        } else {
            self.preview_limits.max(0) as usize
        }
    }
}

/// The operator-pending result computed by an `<expr>` keymap callback and
/// consumed by `:JiebaOmapPlayback`.
///
/// The split is forced: nvim holds `expr_map_lock` while an `<expr>` callback
/// runs, so the callback may not mutate the buffer or the undo history. It
/// therefore only reads, stashes, and returns the `<Esc><Cmd>...<CR>` string.
pub struct PendingOmap {
    /// Equivalent motion name (`w`, `W`, `ge`, `iw`, ...), also used to build
    /// `<Plug>(Jieba_internal_o_{motion})` for vim-repeat.
    pub motion: &'static str,
    pub count: u64,
    pub operator: String,
    pub register: String,
    /// True when re-running a `c` operation via vim-repeat's `.`.
    pub repeat: bool,
    /// Where the cursor should land once the operator has run.
    pub cursor: Pos4,
    pub langle: Pos4,
    pub rangle: Pos4,
    /// `b"v"` or `b"V"`.
    pub visualmode: &'static [u8],
    /// `b"inclusive"` or `b"exclusive"`.
    pub selection: &'static [u8],
    pub prevent_change: bool,
    /// `getcurpos()` as of the `<expr>` callback, needed for the `d`-special
    /// `curswant` repositioning.
    pub orig_curpos: Curpos5,
}

/// The insert-mode action computed by an `<expr>` keymap callback and performed
/// by `:JiebaImapExec`.
pub enum PendingImap {
    /// `cursor(0, col, 0)` - `i_CTRL-W` with the cursor on a virtual column
    /// (`'virtualedit'`), which only has to move the cursor back to a real one.
    KeepLine { lnum: usize, col: usize },
    /// `cursor(lnum, col)` - the insert-mode arrow keys.
    Cursor { lnum: usize, col: usize },
    /// `del_to_cursor(start_col, cur_col)` - `i_CTRL-W` deleting back over a
    /// word.
    Delete { start_col: usize, cur_col: usize },
}

/// Visual state saved by the experimental `g@` path so the self-deleting
/// `CursorMoved` autocmd can put `'<` / `'>` back afterwards.
pub struct LastVisual {
    pub langle: Pos4,
    pub rangle: Pos4,
    /// Raw `visualmode()` bytes - a blockwise selection is `\x16`, not UTF-8.
    /// Empty when visual mode had never been used.
    pub visualmode: Vec<u8>,
    pub model_visualmode: &'static [u8],
}

pub struct State {
    pub opts: RefCell<Rc<Opts>>,
    /// Built once by `setup()`. `None` until then, and `None` again if the
    /// engine could not be constructed (a bad user dictionary), which every
    /// entry point reports rather than panics on.
    pub wm: RefCell<Option<WordMotion<JiebaImpl>>>,
    /// Written by the `OptionSet iskeyword` autocmd, applied by the next motion.
    ///
    /// Deliberately *not* applied inside the autocmd: `iskeyword` can be set by
    /// a user `CursorMoved` autocmd that fires while our own `normal!` playback
    /// is running, i.e. while `wm` is already mutably borrowed. Deferring the
    /// write makes that re-entrancy structurally impossible instead of relying
    /// on a lock.
    pub pending_isk: RefCell<Option<String>>,
    pub pending_op: RefCell<Option<PendingOmap>>,
    pub pending_imap: RefCell<Option<PendingImap>>,
    pub last_visual: RefCell<Option<LastVisual>>,
    /// Namespace id for the motion preview extmarks (0 until created).
    pub preview_ns: Cell<u32>,
    pub augroup: Cell<i64>,
}

impl State {
    pub fn new() -> Self {
        Self {
            opts: RefCell::new(Rc::new(Opts::default())),
            wm: RefCell::new(None),
            pending_isk: RefCell::new(None),
            pending_op: RefCell::new(None),
            pending_imap: RefCell::new(None),
            last_visual: RefCell::new(None),
            preview_ns: Cell::new(0),
            augroup: Cell::new(0),
        }
    }

    pub fn gopts(&self) -> Rc<Opts> {
        Rc::clone(&self.opts.borrow())
    }

    pub fn set_opts(&self, opts: Opts) {
        *self.opts.borrow_mut() = Rc::new(opts);
    }

    /// Construct (or reconstruct) the engine from the current options. Returns
    /// an error message suitable for echoing, or `None` on success.
    pub fn build_word_motion(&self) -> Option<String> {
        let opts = self.gopts();
        let jieba = match JiebaImpl::new(opts.user_dict.clone(), !opts.lazy) {
            Ok(j) => j,
            Err(err) => return Some(format!("jieba_rs: {err}")),
        };
        let isk = crate::nvimrs::get_option_as::<String>("iskeyword", 0, 0)
            .unwrap_or_default();
        match Tokenizer::try_new(jieba, isk.as_bytes()) {
            Ok(tokenizer) => {
                *self.wm.borrow_mut() = Some(WordMotion::new(tokenizer));
                *self.pending_isk.borrow_mut() = None;
                None
            }
            Err(_) => {
                Some(format!("jieba_rs: failed to parse 'iskeyword': {isk}"))
            }
        }
    }

    /// Apply a deferred `'iskeyword'` change, if any.
    fn apply_pending_isk(&self) {
        let isk = self.pending_isk.borrow_mut().take();
        let Some(isk) = isk else { return };
        if let Some(wm) = self.wm.borrow_mut().as_mut()
            && wm
                .get_tokenizer_mut()
                .try_set_word_predicate(isk.as_bytes())
                .is_err()
        {
            crate::nvimrs::echo(&format!(
                "jieba_rs: failed to parse 'iskeyword': {isk}"
            ));
        }
    }

    /// Set `'iskeyword'` explicitly (the `update_isk` export).
    pub fn update_isk(&self, isk: &str) -> Option<String> {
        let mut wm = self.wm.borrow_mut();
        let Some(wm) = wm.as_mut() else {
            return Some("jieba_rs: not set up yet".to_string());
        };
        wm.get_tokenizer_mut()
            .try_set_word_predicate(isk.as_bytes())
            .err()
            .map(|_| format!("jieba_rs: failed to parse 'iskeyword': {isk}"))
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

/// Run `f` against the engine, applying any deferred `'iskeyword'` change
/// first. Returns `None` when the plugin is not set up.
///
/// # Borrow discipline
///
/// The returned guard is a `RefMut`, so **`f` must not call anything that can
/// re-enter the plugin** - no `api::command`, no `normal!`, no autocmd-triggering
/// FFI. Reading buffer lines is safe (`nvim_buf_get_lines` fires no autocmds).
/// Compute an owned result inside `f`, drop the guard, and only then mutate the
/// editor.
pub fn with_wm<R>(
    state: &SharedState,
    f: impl FnOnce(&mut WordMotion<JiebaImpl>) -> R,
) -> Option<R> {
    state.apply_pending_isk();
    let mut wm = state.wm.borrow_mut();
    let wm = wm.as_mut()?;
    Some(f(wm))
}
