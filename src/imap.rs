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

//! Insert-mode keys - the port of `jieba_vim#mapping#imap_expr`.

use nvim_oxi::api;

use crate::state::{PendingImap, SharedState};
use crate::{model, pos, util};

// Neovim's termcodes for these keys, verified byte-for-byte against
// `nvim_replace_termcodes` and matching what `jieba_vim_rs_core::motion::imap`
// dispatches on.
pub const KEY_CTRL_W: &[u8] = b"\x17";
pub const KEY_CTRL_LEFT: &[u8] = b"\x80\xfdU";
pub const KEY_CTRL_RIGHT: &[u8] = b"\x80\xfdV";
pub const KEY_SHIFT_LEFT: &[u8] = b"\x80#4";
pub const KEY_SHIFT_RIGHT: &[u8] = b"\x80%i";

/// Returned for the keys whose action has to run through `:JiebaImapExec`.
pub const RHS_EXEC: &str = "<Cmd>JiebaImapExec<CR>";

/// The `<expr>` callback for `i_CTRL-W` - `s:imap_ctrlw_expr`.
///
/// Read-only, like every `<expr>` callback: it decides *which* action to take
/// and stashes its arguments, then `<Cmd>` performs it.
pub fn ctrl_w_expr(state: &SharedState) -> String {
    let Some(curpos) = pos::getcurpos() else {
        return String::new();
    };
    let [_, lnum, col, off, _] = curpos;

    // On a virtual column ('virtualedit') there is nothing to delete; the
    // builtin just moves the cursor back to a real column on the same line.
    if off > 0 {
        *state.pending_imap.borrow_mut() =
            Some(PendingImap::KeepLine { lnum, col });
        return RHS_EXEC.to_string();
    }
    // At column 1, `CTRL-W` joins with the previous line - exactly what a
    // backspace does.
    if col == 1 {
        return "<BS>".to_string();
    }
    let Some(out) = model::imap(state, KEY_CTRL_W, curpos) else {
        return String::new();
    };
    *state.pending_imap.borrow_mut() = Some(PendingImap::Delete {
        start_col: out.cursor[2],
        cur_col: col,
    });
    RHS_EXEC.to_string()
}

/// The `<expr>` callback for the insert-mode arrow keys - `s:imap_arrow_expr`.
pub fn arrow_expr(state: &SharedState, key: &'static [u8]) -> String {
    let Some(curpos) = pos::getcurpos() else {
        return String::new();
    };
    let Some(out) = model::imap(state, key, curpos) else {
        return String::new();
    };
    *state.pending_imap.borrow_mut() = Some(PendingImap::Cursor {
        lnum: out.cursor[1],
        col: out.cursor[2],
    });
    RHS_EXEC.to_string()
}

/// `:JiebaImapExec` - performs what an `<expr>` callback computed.
///
/// `<Cmd>` runs without leaving insert mode, which is what makes moving the
/// cursor and rewriting the line here equivalent to upstream's
/// `<Cmd>call cursor(...)` / `<Cmd>call del_to_cursor(...)`.
pub fn exec(state: &SharedState) {
    let Some(pending) = state.pending_imap.borrow_mut().take() else {
        return;
    };
    let buf = api::get_current_buf();
    match pending {
        PendingImap::KeepLine { lnum, col } => pos::set_cursor(&buf, lnum, col),
        PendingImap::Cursor { lnum, col } => pos::set_cursor(&buf, lnum, col),
        PendingImap::Delete { start_col, cur_col } => {
            util::del_to_cursor(&buf, start_col, cur_col)
        }
    }
}
