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

//! The experimental `g@` / `'operatorfunc'` path - the port of
//! `jieba_vim#mapping#omap_visual_expr` and `s:omap_visual_restore_visual`.
//!
//! Enabled by `setup({ experimental_opfunc = true })`. The playback strategy in
//! `omap.rs` re-runs the operator itself, which cannot work for an arbitrary
//! `'operatorfunc'` (it would have to re-dispatch `v:operator`, and operators
//! that call `getchar()` - `g@` with a prompting opfunc, mini.surround - would
//! deadlock against the consumed typeahead). This strategy instead selects the
//! range in Visual mode and lets vim hand it to the operator, which supports any
//! `'operatorfunc'` at the cost of possibly clobbering `'<` / `'>`.
//!
//! See `docs/design_choices/0003-omap-impl-tradeoff.md`.

use nvim_oxi::api;
use nvim_oxi::{Array, Object};

use crate::state::{LastVisual, SharedState};
use crate::{nvimrs, pos, util};

/// `:JiebaOmapVisual` - the second pass, running while the operator is still
/// pending (upstream re-entered itself through `<Cmd>` without a leading `<Esc>`
/// for exactly this reason).
///
/// Selects the model's range in Visual mode and leaves it selected, so vim
/// applies the pending `g@` to it.
pub fn visual(state: &SharedState) {
    let Some(p) = state.pending_op.borrow_mut().take() else {
        return;
    };

    // Save `'<` / `'>` and the visual mode so the CursorMoved autocmd below can
    // undo the damage the selection does to them.
    let Some(langle) = pos::getpos("'<") else {
        return;
    };
    let Some(rangle) = pos::getpos("'>") else {
        return;
    };
    *state.last_visual.borrow_mut() = Some(LastVisual {
        langle,
        rangle,
        visualmode: util::visualmode_bytes(),
        model_visualmode: p.visualmode,
    });

    // One-shot, buffer-local: upstream used `autocmd CursorMoved <buffer>` plus a
    // self-deleting `autocmd!`, which is what `once` expresses natively. If a
    // second g@ lands before any CursorMoved, the stale autocmd simply finds no
    // saved state and no-ops.
    let s = std::rc::Rc::clone(state);
    nvimrs::create_autocmd_cb(
        &["CursorMoved"],
        state.augroup.get(),
        "",
        api::get_current_buf().handle(),
        true,
        move |_| {
            crate::guard("opfunc_restore_visual", || restore_visual(&s));
            false
        },
    );

    // Arm the visual mode the model asked for, then leave it so `VIsual_mode` is
    // set for the `gv` below.
    let mut cmd = p.visualmode.to_vec();
    cmd.push(0x1b);
    util::silent_normal_raw(&cmd);

    let forward = util::is_forward_motion(p.motion);
    let (start_pos, end_pos) = if forward {
        (p.langle, p.rangle)
    } else {
        (p.rangle, p.langle)
    };
    pos::setpos("'<", start_pos);
    let mut end_pos = end_pos;
    if p.selection == b"exclusive" {
        // Visual mode is inclusive, so an exclusive range has to give back its
        // last column.
        end_pos[2] = end_pos[2].saturating_sub(1);
    }
    pos::setpos("'>", end_pos);

    util::normal("gv");
}

/// `:JiebaOmapAbort` - the `g@` path's "the motion selected nothing" case.
pub fn abort(state: &SharedState) {
    let Some(p) = state.pending_op.borrow_mut().take() else {
        return;
    };
    util::consume_chars();
    let buf = api::get_current_buf();
    pos::set_cursor(&buf, p.cursor[1], p.cursor[2]);
}

/// `s:omap_visual_restore_visual`.
fn restore_visual(state: &SharedState) {
    let Some(lv) = state.last_visual.borrow_mut().take() else {
        return;
    };
    if lv.visualmode.is_empty() {
        // `visualmode(1)`: visual mode was never used before, so reset the
        // remembered mode rather than restoring one.
        let _ = nvimrs::call_function(
            "visualmode",
            &Array::from_iter([Object::from(1i64)]),
        );
    } else if lv.visualmode.as_slice() != lv.model_visualmode {
        // The selection changed the visual mode; put the old one back.
        let mut cmd = lv.visualmode.clone();
        cmd.push(0x1b);
        util::silent_normal_raw(&cmd);
    }
    pos::setpos("'<", lv.langle);
    pos::setpos("'>", lv.rangle);
}
