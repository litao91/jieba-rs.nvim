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

//! Visual-mode word motions and text objects - the port of
//! `jieba_vim#mapping#xmap`.

use crate::{model, pos, state::SharedState, util};

/// Extend (or shrink) the visual selection by `{motion}`.
///
/// A plain keymap callback runs with visual mode still active, exactly like the
/// `<Cmd>` the upstream `<expr>` mapping returned, so the first step is the same
/// explicit `<Esc>`.
pub fn run(state: &SharedState, motion: &'static [u8]) {
    // `v:count` is reset by the `<Esc>` below, so read it first - upstream
    // captured it while building the `<expr>` mapping's `<Cmd>` string, which is
    // the same point in time.
    let count = util::v_count1();

    util::normal_noautocmd("\x1b");

    // Capture the selection endpoints through marks 'a'/'b', preserving
    // whatever the user had there. `gv o ma o mb` puts the cursor on each end in
    // turn: `gv` re-selects, `o` jumps to the other end.
    let orig_mark_a = pos::getpos("'a");
    let orig_mark_b = pos::getpos("'b");
    util::normal_noautocmd_silent("gvomaomb\x1b");
    let visual_begin = pos::getpos("'a");
    let visual_end = pos::getpos("'b");
    if let Some(m) = orig_mark_a {
        pos::setpos("'a", m);
    }
    if let Some(m) = orig_mark_b {
        pos::setpos("'b", m);
    }

    let vmode = util::visualmode_bytes();
    let (Some(visual_begin), Some(visual_end)) = (visual_begin, visual_end)
    else {
        return;
    };
    let Some(out) =
        model::xmap(state, &vmode, motion, visual_begin, visual_end, count)
    else {
        return;
    };

    // Re-arm the visual mode the model asked for and leave it, which is what
    // makes the following `gv` restore *that* mode; there is no API for setting
    // `VIsual_mode` directly. The mode byte may be a bare \x16 (blockwise),
    // which is not valid UTF-8, so the command is built from bytes.
    let mut cmd = out.visualmode.to_vec();
    cmd.push(0x1b);
    util::normal_noautocmd_raw(&cmd);

    pos::setpos("'<", out.langle);
    pos::setpos("'>", out.rangle);

    if out.visualmode == b"v" && vmode != b"v" {
        // The visual mode actually changed, so let the ModeChanged event through
        // (plugins such as statuslines depend on it).
        util::normal("gv");
    } else {
        util::normal_noautocmd("gv");
    }

    if out.prevent_change {
        util::consume_chars();
    }
}
