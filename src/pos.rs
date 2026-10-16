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

//! Cursor and mark positions.
//!
//! The engine speaks Vim's own position encodings, so these helpers wrap the
//! vimscript accessors rather than the `nvim_win_*` cursor API.
//!
//! `jieba_vim_rs_core`'s `motion::api::ffi::{Position, CursorPositionCurswant}`
//! are private aliases, so the equivalent aliases are declared here. They are
//! structurally identical (`[usize; 4]` / `[usize; 5]`), so they interoperate
//! with the engine without patching the vendored crate.

use nvim_oxi::api::{self, Buffer};
use nvim_oxi::conversion::FromObject;
use nvim_oxi::{Array, Object};

use crate::nvimrs;

/// `getpos()`'s 4-element `[bufnum, lnum, col, off]`; `lnum`/`col` 1-based,
/// `off` 0-based.
pub type Pos4 = [usize; 4];

/// `getcurpos()`'s 5-element `[bufnum, lnum, col, off, curswant]`; `lnum`,
/// `col` and `curswant` 1-based, `off` 0-based.
pub type Curpos5 = [usize; 5];

/// Read an `Array` of nvim Integers into a fixed-size `usize` tuple.
fn array_to_usize<const N: usize>(obj: Object) -> Option<[usize; N]> {
    let arr = Array::from_object(obj).ok()?;
    if arr.len() < N {
        return None;
    }
    let mut out = [0usize; N];
    for (slot, item) in out.iter_mut().zip(arr.iter()) {
        *slot = usize::try_from(i64::try_from(item.clone()).ok()?).ok()?;
    }
    Some(out)
}

/// `getcurpos()` - `[bufnum, lnum, col, off, curswant]`.
pub fn getcurpos() -> Option<Curpos5> {
    let obj = nvimrs::call_function("getcurpos", &Array::new())?;
    array_to_usize::<5>(obj)
}

/// `getpos({expr})` - `[bufnum, lnum, col, off]`. `{expr}` includes the leading
/// quote, e.g. `"'a"`, `"'<"`, `"'>"`, `"v"`.
pub fn getpos(expr: &str) -> Option<Pos4> {
    let obj = nvimrs::call_function(
        "getpos",
        &Array::from_iter([Object::from(expr)]),
    )?;
    array_to_usize::<4>(obj)
}

/// `setpos({expr}, {pos})`.
pub fn setpos(expr: &str, pos: Pos4) {
    let arr = Array::from_iter(pos.into_iter().map(|n| Object::from(n as i64)));
    let _ = nvimrs::call_function(
        "setpos",
        &Array::from_iter([Object::from(expr), Object::from(arr)]),
    );
}

/// `getline(".")` - the current line's text, or `""`.
pub fn cur_line_text() -> String {
    nvimrs::call_fn_as::<String>(
        "getline",
        &Array::from_iter([Object::from(".")]),
    )
    .unwrap_or_default()
}

/// `getline({lnum})` for an arbitrary 1-based line number.
pub fn line_text(lnum: usize) -> String {
    nvimrs::call_fn_as::<String>(
        "getline",
        &Array::from_iter([Object::from(lnum as i64)]),
    )
    .unwrap_or_default()
}

/// `col("$")` - one past the last byte of the current line.
pub fn col_dollar(line: &str) -> usize {
    line.len() + 1
}

/// Move the cursor in the current window to a 1-based `(lnum, col)`, the way
/// vimscript's `cursor()` does: clamp `col` to the line and back off to a UTF-8
/// boundary so a byte column the engine computed for a different line length
/// cannot land mid-character (`nvim_win_set_cursor` would reject it).
pub fn set_cursor(buf: &Buffer, lnum: usize, col: usize) {
    let mut win = api::get_current_win();
    let count = buf.line_count().unwrap_or(0).max(1);
    let lnum = lnum.clamp(1, count);
    let mut col = col.max(1);
    if let Ok(lines) = buf.get_lines(lnum - 1..lnum, false)
        && let Some(line) = lines.into_iter().next()
    {
        let line = line.to_string_lossy();
        if col > line.len() + 1 {
            col = line.len() + 1;
        }
        let mut c0 = col - 1;
        while c0 > 0 && !line.is_char_boundary(c0) {
            c0 -= 1;
        }
        col = c0 + 1;
    }
    let _ = win.set_cursor(lnum, col - 1);
}

/// `virtcol2col(0, {lnum}, {virtcol})` - the byte column at a virtual column
/// (Neovim >= 0.10). Returns 0 when unavailable, matching vimscript's fallback.
pub fn virtcol2col(lnum: usize, virtcol: usize) -> usize {
    let got = nvimrs::call_fn_as::<i64>(
        "virtcol2col",
        &Array::from_iter([
            Object::from(0i64),
            Object::from(lnum as i64),
            Object::from(virtcol as i64),
        ]),
    );
    got.map(|c| c.max(0) as usize).unwrap_or(0)
}
