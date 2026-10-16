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

//! Operator-pending word motions - the port of `jieba_vim#mapping#omap_playback*`.
//!
//! # Why this is an `<expr>` map that returns `<Esc><Cmd>...`
//!
//! A non-`<expr>` Lua keymap rhs is stored as `"<K_LUA><id><CR>"` and dispatches
//! through `nv_colon`, which - when an operator is pending - sets
//! `motion_type = kMTCharWise` / `inclusive = false` (normal.c) and then runs
//! `do_pending_operator` as soon as the callback returns `true`. `map_execute_lua`
//! returns true regardless of what the callback did, so the *real* operator would
//! fire over a zero-width range before any keys we feed are consumed: `yw` would
//! report `E353: Nothing in register`, `cw` would drop straight into insert
//! mode, `dw` would clobber `'[`/`']`.
//!
//! So the map stays `<expr>` and returns the same two-step sequence upstream's
//! vimscript did: `<Esc>` aborts the pending operator (`nv_esc` -> `clearop`),
//! then `<Cmd>` runs our playback in ordinary normal mode.
//!
//! nvim holds `expr_map_lock` while an `<expr>` callback runs, which makes
//! `:normal` fail with E943 and `u_save` refuse. The callback therefore only
//! *reads* - it computes the model output, stashes it in [`PendingOmap`], and
//! returns. All mutation happens in the `:JiebaOmapPlayback` command callback.

use nvim_oxi::api;
use nvim_oxi::{Array, Object};

use crate::state::{PendingOmap, SharedState};
use crate::{model, nvimrs, pos, util};

/// Returned by every operator-pending `<expr>` callback; `replace_keycodes` is on
/// so nvim translates the markup into real keycodes.
pub const RHS_PLAYBACK: &str = "<Esc><Cmd>JiebaOmapPlayback<CR>";
/// The experimental `g@` path re-enters with visual mode still selected, so it
/// must *not* abort the operator first.
pub const RHS_VISUAL: &str = "<Cmd>JiebaOmapVisual<CR>";
/// The experimental path's "motion failed" case still has to abort.
pub const RHS_ABORT: &str = "<Esc><Cmd>JiebaOmapAbort<CR>";

/// The `<expr>` callback for `<Plug>(Jieba_{motion})` in operator-pending mode.
///
/// `repeat` is true only for the internal `<Plug>(Jieba_internal_o_{motion})`
/// maps that vim-repeat's `.` replays; it makes `c` re-insert `@.` instead of
/// starting an empty insert.
///
/// Read-only by necessity: see the module docs.
pub fn expr_cb(
    state: &SharedState,
    motion: &'static str,
    repeat: bool,
) -> String {
    let Some(orig_curpos) = pos::getcurpos() else {
        // Cannot even read the cursor; abort the operator rather than hang.
        return "\x1b".to_string();
    };
    let count = util::v_count1();
    let operator = util::v_operator();
    let register = util::v_register();

    let Some(out) = model::omap(
        state,
        motion.as_bytes(),
        orig_curpos,
        count,
        operator.as_bytes(),
    ) else {
        return "\x1b".to_string();
    };

    let gopts = state.gopts();
    let via_opfunc = gopts.experimental_opfunc && operator == "g@";

    *state.pending_op.borrow_mut() = Some(PendingOmap {
        motion,
        count,
        operator,
        register,
        repeat,
        cursor: out.cursor,
        langle: out.langle,
        rangle: out.rangle,
        visualmode: out.visualmode,
        selection: out.selection,
        prevent_change: out.prevent_change,
        orig_curpos,
    });

    if via_opfunc {
        // Upstream's omap_visual_expr falls back to the playback path for every
        // operator but g@, and aborts early when the motion selected nothing.
        if out.prevent_change {
            RHS_ABORT
        } else {
            RHS_VISUAL
        }
    } else {
        RHS_PLAYBACK
    }
    .to_string()
}

/// The vim-repeat sequence `{operator}<Plug>(Jieba_internal_o_{motion})>`, with
/// `<Plug>` expanded to the real `K_PLUG` keycode. Those bytes are not valid
/// UTF-8, so the sequence is kept as bytes end to end.
fn repeat_seq(motion: &str, operator: &str) -> Vec<u8> {
    let mut seq = operator.as_bytes().to_vec();
    seq.extend_from_slice(b"<Plug>(Jieba_internal_o_");
    seq.extend_from_slice(motion.as_bytes());
    seq.push(b')');
    util::termcodes_bytes(&seq)
}

/// `:JiebaOmapPlayback` - the port of `jieba_vim#mapping#omap_playback`.
pub fn playback(state: &SharedState) {
    let Some(p) = state.pending_op.borrow_mut().take() else {
        return;
    };
    let use_repeat = !p.prevent_change && p.operator != "y";
    let seq = repeat_seq(p.motion, &p.operator);
    if use_repeat {
        // A missing vim-repeat just sets the call's error, which `call_function`
        // already turns into None - the equivalent of upstream's `silent!`.
        util::repeat_setreg(&seq, &p.register);
    }
    playback_core(&p);
    if use_repeat {
        util::repeat_set(&seq, p.count);
    }
}

/// The port of `s:omap_playback_core`.
///
/// Rather than asking vim to run the operator over a motion (which would recurse
/// back into this plugin), this drives the operator itself: it positions the
/// cursor at the start of the range and then replays `{operator}` with either an
/// explicit line count (linewise) or a `\`a` jump to a stashed end mark
/// (characterwise).
fn playback_core(p: &PendingOmap) {
    let buf = api::get_current_buf();
    pos::set_cursor(&buf, p.langle[1], p.langle[2]);

    if p.prevent_change {
        // The motion failed: land where the model said and swallow the
        // operator's leftover typeahead.
        pos::set_cursor(&buf, p.cursor[1], p.cursor[2]);
        util::consume_chars();
        return;
    }

    if p.operator != "y" {
        // A no-op setline that installs an undoable checkpoint, so a single `u`
        // undoes the whole operation rather than just the operator's own edit.
        let line = pos::cur_line_text();
        let _ = nvimrs::call_function(
            "setline",
            &Array::from_iter([Object::from("."), Object::from(line)]),
        );
    }

    let orig_mark_a = pos::getpos("'a");
    let orig_startofline = opt_bool("startofline");
    let orig_eventignore = nvimrs::get_option_as::<String>("eventignore", 0, 0)
        .unwrap_or_default();

    // 'startofline' has to be on for the cursor to land correctly after a
    // linewise `d`; the events are suppressed to match what the builtin motions
    // do (a real `dw` does not fire InsertEnter/InsertLeave or ModeChanged).
    let _ = nvimrs::set_option_value("startofline", Object::from(true), 0, 0);
    let _ = nvimrs::set_option_value(
        "eventignore",
        Object::from("InsertEnter,InsertLeave,ModeChanged"),
        0,
        0,
    );

    // Select ...
    let forward = util::is_forward_motion(p.motion);
    let (start_pos, end_pos) = if forward {
        (p.langle, p.rangle)
    } else {
        (p.rangle, p.langle)
    };
    pos::set_cursor(&buf, start_pos[1], start_pos[2]);

    // Needed later to decide whether to reposition after a linewise `d` when
    // 'startofline' is off: only a non-empty end line leaves something to land
    // on.
    let need_repos = !pos::line_text(end_pos[1]).is_empty();

    // ... and execute.
    let cont = if p.operator == "c" && p.repeat {
        // `@.` is the text the previous `c` inserted; read it, never interpolate
        // a vimscript expression.
        util::getreg(".")
    } else {
        String::new()
    };
    if p.visualmode == b"V" {
        // Linewise: `{count}{op}{op}` with an explicit line count. i64 because
        // usize would wrap if the model ever returned end < start.
        let op_lines = end_pos[1] as i64 - start_pos[1] as i64 + 1;
        util::normal(&format!(
            "\"{}{}{}{}{}",
            p.register, op_lines, p.operator, p.operator, cont
        ));
    } else {
        // Characterwise: stash the end in 'a and jump to it, so the operator's
        // own inclusive/exclusive rule applies via the forced `v` motion.
        let forced = if p.selection == b"inclusive" { "v" } else { "" };
        pos::setpos("'a", end_pos);
        util::normal(&format!(
            "\"{}{}{}`a{}",
            p.register, p.operator, forced, cont
        ));
    }

    // Restore the options and the mark we borrowed.
    let _ = nvimrs::set_option_value(
        "eventignore",
        Object::from(orig_eventignore),
        0,
        0,
    );
    let _ = nvimrs::set_option_value(
        "startofline",
        Object::from(orig_startofline),
        0,
        0,
    );
    if let Some(m) = orig_mark_a {
        pos::setpos("'a", m);
    }

    // Land the cursor. For a linewise operation vim already placed it (and for a
    // `.`-replayed `c` we are about to enter insert mode, so leave it alone).
    if p.visualmode != b"V" && !p.repeat {
        pos::set_cursor(&buf, p.cursor[1], p.cursor[2]);
    }

    // A linewise `d` with 'startofline' off: vim put the cursor at the start of
    // the joined line, but the builtin behaviour keeps the original virtual
    // column, so restore it.
    if !opt_bool("startofline") && need_repos && p.visualmode == b"V" {
        let lnum = util::cur_lnum();
        let col = pos::virtcol2col(lnum, p.orig_curpos[4]);
        pos::set_cursor(&buf, lnum, col);
    }

    // `c` has to drop the user in insert mode; `startinsert!` when the motion
    // reached end of line (the builtin `c$` behaviour).
    if p.operator == "c" && !p.repeat {
        let at_eol = p.cursor[2] >= pos::col_dollar(&pos::cur_line_text());
        let _ = api::command(if at_eol {
            "startinsert!"
        } else {
            "startinsert"
        });
    }
}

/// Read a boolean option, tolerating either an nvim Boolean or an Integer.
fn opt_bool(name: &str) -> bool {
    if let Some(b) = nvimrs::get_option_as::<bool>(name, 0, 0) {
        return b;
    }
    nvimrs::get_option_as::<i64>(name, 0, 0).unwrap_or(0) != 0
}
