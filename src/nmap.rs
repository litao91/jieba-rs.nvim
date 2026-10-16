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

//! Normal-mode word motions - the port of `jieba_vim#mapping#nmap`.

use nvim_oxi::api;

use crate::{model, pos, state::SharedState, util};

/// Run `{motion}` from the cursor. A plain (non-`<expr>`) keymap callback is
/// enough here: nothing is pending in normal mode, so moving the cursor
/// directly is safe (unlike operator-pending, which see `omap.rs`).
pub fn run(state: &SharedState, motion: &'static [u8]) {
    let Some(curpos) = pos::getcurpos() else {
        return;
    };
    let count = util::v_count1();
    let Some(out) = model::nmap(state, motion, curpos, count) else {
        return;
    };
    // The borrow on the engine is gone by now, so mutating the editor is safe.
    let buf = api::get_current_buf();
    pos::set_cursor(&buf, out.cursor[1], out.cursor[2]);
    if out.prevent_change {
        // The motion failed (e.g. at the buffer edge); drop any typeahead that
        // was meant for a successful one so it cannot run as normal commands.
        util::consume_chars();
    }
}
