// Copyright 2024-2026 Kaiwen Wu. All Rights Reserved.
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

//! Word-motion preview - the port of `jieba_vim#mapping#preview`, rendered with
//! extmarks instead of `:match`.
//!
//! Upstream looped `execute 'match JiebaPreview /.../'` once per position, but
//! `:match` is one-slot-per-window, so every iteration but the last was a no-op
//! and only the final position was ever highlighted. Extmarks highlight all of
//! them, and clearing them is a real cancel (upstream's `hi clear` only made the
//! leftover match invisible).

use nvim_oxi::api::{self, opts::SetExtmarkOpts};

use crate::buffer::NvimBuffer;
use crate::state::{SharedState, with_wm};
use crate::{nvimrs, pos};

const NAMESPACE: &str = "jieba_rs_preview";

/// Collect the cursor positions `{motion}` would land on, starting from the
/// current cursor.
fn positions(
    state: &SharedState,
    motion: &[u8],
    limit: usize,
) -> Option<Vec<(usize, usize)>> {
    let curpos = pos::getcurpos()?;
    positions_at(state, motion, curpos, limit as i64)
}

/// Collect the cursor positions `{motion}` would land on, starting from an
/// explicit `curpos`.
///
/// With `limit == 0` only the current line is walked (stopping at the first
/// step that changes line or makes no progress); otherwise at most `limit`
/// steps are collected, stopping when a step makes no progress.
pub fn positions_at(
    state: &SharedState,
    motion: &[u8],
    curpos: pos::Curpos5,
    limit: i64,
) -> Option<Vec<(usize, usize)>> {
    // Upstream read `g:jieba_vim_preview_limits` and treated a negative value as
    // unbounded; the exported call takes the bound directly.
    let limit = if limit < 0 {
        99999usize
    } else {
        limit as usize
    };
    let buffer = NvimBuffer::new(api::get_current_buf());
    let start = (curpos[1], curpos[2]);

    with_wm(state, |wm| {
        let mut positions = Vec::new();
        let mut at = start;
        if limit == 0 {
            loop {
                let out = wm
                    .nmap(&buffer, motion, [0, at.0, at.1, 0, at.1], 1)
                    .ok()?;
                let next = (out.cursor[1], out.cursor[2]);
                // Reached either beginning of file or end of file.
                if next == at {
                    break;
                }
                // Reached either previous line or next line.
                if next.0 != at.0 {
                    break;
                }
                positions.push(next);
                at = next;
            }
        } else {
            while positions.len() < limit {
                let out = wm
                    .nmap(&buffer, motion, [0, at.0, at.1, 0, at.1], 1)
                    .ok()?;
                let next = (out.cursor[1], out.cursor[2]);
                // Reached either beginning of file or end of file.
                if next == at {
                    break;
                }
                positions.push(next);
                at = next;
            }
        }
        Some(positions)
    })
    .flatten()
}

fn namespace(state: &SharedState) -> u32 {
    let ns = state.preview_ns.get();
    if ns != 0 {
        return ns;
    }
    // `nvim_create_namespace` is idempotent by name.
    let ns = api::create_namespace(NAMESPACE);
    state.preview_ns.set(ns);
    ns
}

/// Highlight where `{motion}` would land. `:JiebaPreviewCancel` undoes it.
pub fn show(state: &SharedState, motion: &[u8]) {
    let limit = state.gopts().preview_limit();
    let Some(found) = positions(state, motion, limit) else {
        cancel(state);
        return;
    };
    let ns = namespace(state);
    let mut buf = api::get_current_buf();
    let _ = buf.clear_namespace(ns, ..);
    if found.is_empty() {
        return;
    }
    // Upstream used a plain `hi link`, i.e. it overrode any user definition.
    nvimrs::set_hl_link("JiebaPreview", "IncSearch", false);

    for (lnum, col) in found {
        let line = pos::line_text(lnum);
        let start = (col.saturating_sub(1)).min(line.len());
        // Highlight the whole character at that column, as `:match /\v%{col}c/`
        // did, rather than a single byte of a multibyte one.
        let width = line[start..].chars().next().map_or(1, |c| c.len_utf8());
        let opts = SetExtmarkOpts::builder()
            .hl_group("JiebaPreview")
            .end_row(lnum.saturating_sub(1))
            .end_col(start + width)
            .build();
        let _ = buf.set_extmark(ns, lnum.saturating_sub(1), start, &opts);
    }
}

/// Clear the preview highlights.
pub fn cancel(state: &SharedState) {
    let ns = state.preview_ns.get();
    if ns == 0 {
        return;
    }
    let mut buf = api::get_current_buf();
    let _ = buf.clear_namespace(ns, ..);
}
