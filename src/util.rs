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

//! Vimscript primitives the ported mapping logic still needs, and the small
//! helpers built on them.

use nvim_oxi::api::{self, Buffer};
use nvim_oxi::{Array, Object};

use crate::nvimrs;
use crate::pos;

/// `execute 'normal! {cmd}'`.
///
/// `:normal` takes its argument literally (no termcode replacement) and `ex_normal`
/// escapes embedded `0x80` bytes itself. A newline in `{cmd}` would still split
/// the command - the same limitation upstream's `execute 'normal! ...'` has.
pub fn normal(cmd: &str) {
    let _ = api::command(&format!("normal! {cmd}"));
}

/// `execute 'normal! {cmd}'` for a command that may contain bytes which are not
/// valid UTF-8 - a blockwise `visualmode()` is the single byte `\x16`, which
/// cannot be carried by `api::command`.
pub fn normal_raw(cmd: &[u8]) {
    let mut full = Vec::with_capacity(cmd.len() + 8);
    full.extend_from_slice(b"normal! ");
    full.extend_from_slice(cmd);
    nvimrs::command_raw(&full);
}

/// `noautocmd execute 'normal! {cmd}'`.
pub fn normal_noautocmd(cmd: &str) {
    let _ = api::command(&format!("noautocmd normal! {cmd}"));
}

/// `noautocmd silent execute 'normal! {cmd}'`.
pub fn normal_noautocmd_silent(cmd: &str) {
    let _ = api::command(&format!("noautocmd silent normal! {cmd}"));
}

/// `noautocmd execute 'normal! {cmd}'` for a `{cmd}` that may contain bytes
/// which are not valid UTF-8.
pub fn normal_noautocmd_raw(cmd: &[u8]) {
    let mut full = Vec::with_capacity(cmd.len() + 20);
    full.extend_from_slice(b"noautocmd normal! ");
    full.extend_from_slice(cmd);
    nvimrs::command_raw(&full);
}

/// `silent! execute 'normal! {cmd}'` for a `{cmd}` that may contain bytes which
/// are not valid UTF-8.
pub fn silent_normal_raw(cmd: &[u8]) {
    let mut full = Vec::with_capacity(cmd.len() + 17);
    full.extend_from_slice(b"silent! normal! ");
    full.extend_from_slice(cmd);
    nvimrs::command_raw(&full);
}

/// Wrap raw bytes in an nvim String object without requiring them to be UTF-8.
pub fn obj_bytes(b: &[u8]) -> Object {
    Object::from(nvim_oxi::String::from_bytes(b))
}

/// `nvim_replace_termcodes({s}, true, true, true)` as raw bytes - the equivalent
/// of vimscript's `"\<...>"` string interpolation. Needed for `<Plug>` sequences
/// handed to vim-repeat, which must carry the real `K_PLUG` keycode, and for
/// anything else that may not be UTF-8.
pub fn termcodes_bytes(s: &[u8]) -> Vec<u8> {
    let args = Array::from_iter([
        Object::from(nvim_oxi::String::from_bytes(s)),
        Object::from(true),
        Object::from(true),
        Object::from(true),
    ]);
    let Some(obj) = nvimrs::call_function("nvim_replace_termcodes", &args)
    else {
        return s.to_vec();
    };
    nvimrs::object_bytes(&obj).unwrap_or_else(|| s.to_vec())
}

/// `visualmode()` as raw bytes: blockwise visual is `\x16`, which is not valid
/// UTF-8 and so cannot come back through `String::from_object`.
pub fn visualmode_bytes() -> Vec<u8> {
    let Some(obj) = nvimrs::call_function("visualmode", &Array::new()) else {
        return Vec::new();
    };
    nvimrs::object_bytes(&obj).unwrap_or_default()
}

/// `v:count1`.
pub fn v_count1() -> u64 {
    nvimrs::get_vvar_as::<i64>("count1").unwrap_or(1).max(1) as u64
}

/// `v:operator`.
pub fn v_operator() -> String {
    nvimrs::get_vvar_as::<String>("operator").unwrap_or_default()
}

/// `v:register`.
pub fn v_register() -> String {
    let reg = nvimrs::get_vvar_as::<String>("register").unwrap_or_default();
    // vimscript's `v:register` is `"` when no register was specified; upstream
    // passed it through verbatim and `normal! "{reg}...` accepted `"`.
    if reg.is_empty() {
        "\"".to_string()
    } else {
        reg
    }
}

/// `getreg({name})`.
pub fn getreg(name: &str) -> String {
    nvimrs::call_fn_as::<String>(
        "getreg",
        &Array::from_iter([Object::from(name)]),
    )
    .unwrap_or_default()
}

/// `line(".")`.
pub fn cur_lnum() -> usize {
    nvimrs::call_fn_as::<i64>("line", &Array::from_iter([Object::from(".")]))
        .map(|n| n.max(0) as usize)
        .unwrap_or(1)
}

/// Port of `jieba_vim#utils#is_forward_motion`: `w`/`e`/`iw`/`aw`
/// case-insensitively, i.e. also `W`/`E`/`iW`/`aW`. Everything else (`b`, `B`,
/// `ge`, `gE`) is backward.
pub fn is_forward_motion(motion: &str) -> bool {
    motion.eq_ignore_ascii_case("w")
        || motion.eq_ignore_ascii_case("e")
        || motion.eq_ignore_ascii_case("iw")
        || motion.eq_ignore_ascii_case("aw")
}

/// Port of `jieba_vim#utils#del_to_cursor`: delete the text between byte columns
/// `start_col` and `cur_col` (both 1-based) of the current line and leave the
/// cursor at `start_col`. Used by insert-mode `CTRL-W`.
pub fn del_to_cursor(buf: &Buffer, start_col: usize, cur_col: usize) {
    let line = pos::cur_line_text();
    let head = if start_col > 1 {
        slice_or_empty(&line, 0, start_col - 1)
    } else {
        ""
    };
    let tail = if cur_col < pos::col_dollar(&line) {
        slice_or_empty(&line, cur_col - 1, line.len())
    } else {
        ""
    };
    let modified = format!("{head}{tail}");
    let _ = nvimrs::call_function(
        "setline",
        &Array::from_iter([Object::from("."), Object::from(modified)]),
    );
    // `cursor(0, start_col)` - keep the line, move the column.
    let lnum = cur_lnum();
    pos::set_cursor(buf, lnum, start_col);
}

/// Vim's `str[a:b]` byte slice, snapped back to a UTF-8 boundary rather than
/// panicking when a caller passes a mid-character column.
fn slice_or_empty(s: &str, start: usize, end: usize) -> &str {
    let mut start = start.min(s.len());
    while start > 0 && !s.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = end.min(s.len());
    while end > start && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(start..end).unwrap_or("")
}

/// Port of `jieba_vim#utils#consume_chars`: drain pending typeahead.
///
/// Used after a motion that had to be aborted (`prevent_change`), so the
/// operator's remaining keystrokes do not land as normal-mode commands. `getchar(1)`
/// returns immediately - `0` when nothing is pending, otherwise the character -
/// so this terminates as soon as the input queue is empty.
///
/// Testing against 27 (`<Esc>`) is load-bearing: consuming a pending `<Esc>`
/// here crashes vim, as upstream's comment records.
pub fn consume_chars() {
    loop {
        let Some(ch) = nvimrs::call_function(
            "getchar",
            &Array::from_iter([Object::from(1i64)]),
        ) else {
            break;
        };
        match i64::try_from(ch) {
            Ok(0) | Ok(27) => break,
            // A String result is a special key name, i.e. something *is*
            // pending; consume it and keep draining.
            _ => {
                let _ = nvimrs::call_function(
                    "getchar",
                    &Array::from_iter([Object::from(0i64)]),
                );
            }
        }
    }
}

/// `silent! call repeat#setreg({seq}, {reg})`. A missing vim-repeat simply sets
/// the call's error, which `call_function` already turns into `None`.
///
/// `{seq}` carries raw `K_PLUG` bytes, so it cannot be a Rust `String`.
pub fn repeat_setreg(seq: &[u8], reg: &str) {
    let _ = nvimrs::call_function(
        "repeat#setreg",
        &Array::from_iter([obj_bytes(seq), Object::from(reg)]),
    );
}

/// `silent! call repeat#set({seq}, {count})`.
pub fn repeat_set(seq: &[u8], count: u64) {
    let _ = nvimrs::call_function(
        "repeat#set",
        &Array::from_iter([obj_bytes(seq), Object::from(count as i64)]),
    );
}
