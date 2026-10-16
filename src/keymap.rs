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

//! Mapping, command and autocmd registration - the port of `plugin/jieba_vim.vim`.
//!
//! Everything here is driven from Rust callbacks; there is no vimscript and no
//! Lua in the mapping path. `setup()` is idempotent, so calling it again
//! reconfigures rather than stacking duplicates.

use std::rc::Rc;

use nvim_oxi::api;

use crate::state::SharedState;
use crate::{imap, nmap, nvimrs, omap, opfunc, preview, xmap};

/// Word motions, mapped in normal, visual and operator-pending mode.
/// `(name, default lhs, description)` - the name is also the engine's motion key.
pub const WORD_MOTIONS: [(&str, &str); 8] = [
    ("w", "w"),
    ("W", "W"),
    ("e", "e"),
    ("E", "E"),
    ("b", "b"),
    ("B", "B"),
    ("ge", "ge"),
    ("gE", "gE"),
];

/// Word text objects, mapped in visual and operator-pending mode only (text
/// objects have no normal-mode form by definition).
pub const TEXT_OBJECTS: [(&str, &str); 4] =
    [("iw", "iw"), ("iW", "iW"), ("aw", "aw"), ("aW", "aW")];

/// Arrow keys. `(name, default lhs, equivalent engine motion, insert-mode key)`
///
/// Normal/visual/operator-pending mode dispatch on the *equivalent* motion,
/// exactly as upstream's `nmap_expr` translated `\<C-Left>` to `"B"`; insert mode
/// passes the real termcode, which is what `WordMotion::imap` matches on.
pub const ARROWS: [(&str, &str, &str, &[u8]); 4] = [
    ("C_Left", "<C-Left>", "B", imap::KEY_CTRL_LEFT),
    ("S_Left", "<S-Left>", "b", imap::KEY_SHIFT_LEFT),
    ("C_Right", "<C-Right>", "W", imap::KEY_CTRL_RIGHT),
    ("S_Right", "<S-Right>", "w", imap::KEY_SHIFT_RIGHT),
];

const AUGROUP: &str = "jieba_rs";

fn plug(name: &str) -> String {
    format!("<Plug>(Jieba_{name})")
}

fn internal_plug(name: &str) -> String {
    format!("<Plug>(Jieba_internal_o_{name})")
}

/// An `<expr>` operator-pending callback, falling back to `<Esc>` if the engine
/// panicked - an empty string would leave the operator pending and hang nvim.
fn expr_omap(
    state: &SharedState,
    motion: &'static str,
    repeat: bool,
) -> impl Fn() -> String + 'static {
    let s = Rc::clone(state);
    move || {
        let rhs = crate::guard("omap", || omap::expr_cb(&s, motion, repeat));
        if rhs.is_empty() {
            "\x1b".to_string()
        } else {
            rhs
        }
    }
}

fn map_normal(state: &SharedState, name: &str, motion: &'static [u8]) {
    let s = Rc::clone(state);
    nvimrs::set_keymap_cb(
        0,
        "n",
        &plug(name),
        "jieba: word motion",
        move || {
            crate::guard("nmap", || nmap::run(&s, motion));
        },
    );
}

fn map_visual(state: &SharedState, name: &str, motion: &'static [u8]) {
    let s = Rc::clone(state);
    nvimrs::set_keymap_cb(
        0,
        "x",
        &plug(name),
        "jieba: word motion",
        move || {
            crate::guard("xmap", || xmap::run(&s, motion));
        },
    );
}

fn map_op(state: &SharedState, name: &str, motion: &'static str) {
    nvimrs::set_keymap_expr_cb(
        0,
        "o",
        &plug(name),
        "jieba: word motion (operator-pending)",
        expr_omap(state, motion, false),
    );
}

/// The maps vim-repeat's `.` replays: same as `map_op` but with `repeat = true`,
/// which makes `c` re-insert `@.` instead of starting an empty insert.
fn map_op_internal(state: &SharedState, motion: &'static str) {
    nvimrs::set_keymap_expr_cb(
        0,
        "o",
        &internal_plug(motion),
        "jieba: internal, used by vim-repeat's `.`",
        expr_omap(state, motion, true),
    );
}

fn map_insert_arrow(state: &SharedState, name: &str, key: &'static [u8]) {
    let s = Rc::clone(state);
    nvimrs::set_keymap_expr_cb(
        0,
        "i",
        &plug(name),
        "jieba: word motion (insert)",
        move || crate::guard("imap_arrow", || imap::arrow_expr(&s, key)),
    );
}

fn map_preview(state: &SharedState, name: &str, motion: &'static [u8]) {
    let s = Rc::clone(state);
    let lhs = format!("<Plug>(Jieba_preview_{name})");
    nvimrs::set_keymap_cb(
        0,
        "n",
        &lhs,
        "jieba: preview word motion",
        move || {
            crate::guard("preview", || preview::show(&s, motion));
        },
    );
}

/// Register every `<Plug>(Jieba_*)` map, the internal repeat maps, the user
/// commands and the autocmds. Always global; the *default* keymaps are what
/// `filetypes` scopes.
pub fn setup_plugs(state: &SharedState) {
    for (name, _) in WORD_MOTIONS {
        map_normal(state, name, name.as_bytes());
        map_visual(state, name, name.as_bytes());
        map_op(state, name, name);
        map_op_internal(state, name);
        map_preview(state, name, name.as_bytes());
    }
    for (name, _) in TEXT_OBJECTS {
        map_visual(state, name, name.as_bytes());
        map_op(state, name, name);
        map_op_internal(state, name);
    }
    for (name, _, equiv, key) in ARROWS {
        map_normal(state, name, equiv.as_bytes());
        map_visual(state, name, equiv.as_bytes());
        map_op(state, name, equiv);
        map_insert_arrow(state, name, key);
    }
    // i_CTRL-W
    let s = Rc::clone(state);
    nvimrs::set_keymap_expr_cb(
        0,
        "i",
        "<Plug>(Jieba_C_w)",
        "jieba: delete word backwards (insert)",
        move || crate::guard("imap_ctrl_w", || imap::ctrl_w_expr(&s)),
    );

    let s = Rc::clone(state);
    nvimrs::set_keymap_cb(
        0,
        "n",
        "<Plug>(Jieba_preview_cancel)",
        "jieba: cancel word motion preview",
        move || {
            crate::guard("preview_cancel", || preview::cancel(&s));
        },
    );

    setup_commands(state);
    setup_autocmds(state);
}

fn setup_commands(state: &SharedState) {
    let s = Rc::clone(state);
    nvimrs::create_user_command_cb(
        "JiebaPreviewCancel",
        "Cancel the word-motion preview",
        move |_| {
            crate::guard("cmd_JiebaPreviewCancel", || preview::cancel(&s));
        },
    );
    // The three below are internal plumbing: an operator-pending `<expr>` map
    // may not mutate the editor (nvim holds expr_map_lock), so it computes and
    // stashes, then returns `<Esc><Cmd>...<CR>` to do the work in normal mode.
    let s = Rc::clone(state);
    nvimrs::create_user_command_cb(
        "JiebaOmapPlayback",
        "jieba-rs.nvim internal: run the pending operator-pending motion",
        move |_| {
            crate::guard("cmd_JiebaOmapPlayback", || omap::playback(&s));
        },
    );
    let s = Rc::clone(state);
    nvimrs::create_user_command_cb(
        "JiebaOmapVisual",
        "jieba-rs.nvim internal: select the pending g@ range visually",
        move |_| {
            crate::guard("cmd_JiebaOmapVisual", || opfunc::visual(&s));
        },
    );
    let s = Rc::clone(state);
    nvimrs::create_user_command_cb(
        "JiebaOmapAbort",
        "jieba-rs.nvim internal: abort a failed g@ motion",
        move |_| {
            crate::guard("cmd_JiebaOmapAbort", || opfunc::abort(&s));
        },
    );
    let s = Rc::clone(state);
    nvimrs::create_user_command_cb(
        "JiebaImapExec",
        "jieba-rs.nvim internal: run the pending insert-mode key action",
        move |_| {
            crate::guard("cmd_JiebaImapExec", || imap::exec(&s));
        },
    );
}

fn setup_autocmds(state: &SharedState) {
    // `clear = true` makes setup() re-runnable without stacking callbacks.
    let group = nvimrs::create_augroup(AUGROUP, true).unwrap_or(0);
    state.augroup.set(group);

    // Track 'iskeyword' so word boundaries follow the user's option. The handler
    // only records the new value: `iskeyword` can be set by a user CursorMoved
    // autocmd that fires while our own operator playback is running, i.e. while
    // the engine is already mutably borrowed, so applying it here would panic.
    let s = Rc::clone(state);
    nvimrs::create_autocmd_cb(
        &["OptionSet"],
        group,
        "iskeyword",
        0,
        false,
        move |_| {
            let isk = nvimrs::get_option_as::<String>("iskeyword", 0, 0)
                .unwrap_or_default();
            *s.pending_isk.borrow_mut() = Some(isk);
            false
        },
    );

    // Scope the default keymaps to filetypes when asked to.
    let filetypes = state.gopts().filetypes.clone();
    if filetypes.is_empty() {
        return;
    }
    let pattern = filetypes.join(",");
    let s = Rc::clone(state);
    nvimrs::create_autocmd_cb(
        &["FileType"],
        group,
        &pattern,
        0,
        false,
        move |_| {
            crate::guard("filetype_keymaps", || {
                default_keymaps(&s, api::get_current_buf().handle());
            });
            false
        },
    );
    // A buffer of one of those filetypes may already be open; the autocmd only
    // fires on the transition.
    let buf = api::get_current_buf();
    let ft: String =
        nvimrs::get_option_as("filetype", buf.handle(), 0).unwrap_or_default();
    if filetypes.contains(&ft) {
        default_keymaps(state, buf.handle());
    }
}

/// Install the default keymaps (`w` -> `<Plug>(Jieba_w)`, ...).
///
/// `buf` = 0 installs them globally, matching upstream's `g:jieba_vim_keymap`;
/// a buffer handle installs them buffer-locally, which is how `filetypes` keeps
/// the plugin out of other buffers.
///
/// These are remappable on purpose: their rhs is a `<Plug>` name, and a
/// `noremap` mapping would never resolve it.
pub fn default_keymaps(state: &SharedState, buf: i32) {
    if !state.gopts().mappings {
        return;
    }
    for (name, lhs) in WORD_MOTIONS {
        for mode in ["n", "x", "o"] {
            nvimrs::set_keymap_remap(
                buf,
                mode,
                lhs,
                &plug(name),
                "jieba: word motion",
            );
        }
    }
    for (name, lhs) in TEXT_OBJECTS {
        for mode in ["x", "o"] {
            nvimrs::set_keymap_remap(
                buf,
                mode,
                lhs,
                &plug(name),
                "jieba: word text object",
            );
        }
    }
    for (name, lhs, _, _) in ARROWS {
        for mode in ["n", "x", "o"] {
            nvimrs::set_keymap_remap(
                buf,
                mode,
                lhs,
                &plug(name),
                "jieba: word motion",
            );
        }
    }
    nvimrs::set_keymap_remap(
        buf,
        "i",
        "<C-w>",
        "<Plug>(Jieba_C_w)",
        "jieba: delete word backwards (insert)",
    );
}
