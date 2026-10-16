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

//! Engine calls and their outputs - the port of `plugin/jieba_vim/model.vim`.
//!
//! Each function here builds a fresh [`NvimBuffer`] snapshot, runs the engine
//! under [`with_wm`], and returns an **owned** result, so the mutable borrow on
//! the engine never outlives the call. Callers then mutate the editor with no
//! borrow held.

use nvim_oxi::api;

use jieba_vim_rs_core::motion::{
    ImapOutput, NmapOutput, OmapOutput, XmapOutput,
};

use crate::buffer::NvimBuffer;
use crate::pos::{Curpos5, Pos4};
use crate::state::{SharedState, with_wm};

/// `ffi::*Output` carry `prevent_change` as the byte string `b"1"` / `b"0"`.
fn flag(bytes: &[u8]) -> bool {
    bytes == b"1"
}

pub struct NmapOut {
    pub cursor: Pos4,
    pub prevent_change: bool,
}

impl From<NmapOutput> for NmapOut {
    fn from(o: NmapOutput) -> Self {
        Self {
            cursor: o.cursor,
            prevent_change: flag(o.prevent_change),
        }
    }
}

pub struct XmapOut {
    pub langle: Pos4,
    pub rangle: Pos4,
    /// Raw `visualmode()` bytes, echoed back by the engine: `b"v"`, `b"V"` or
    /// the single byte `\x16` for blockwise - not valid UTF-8, hence bytes.
    pub visualmode: &'static [u8],
    pub prevent_change: bool,
}

impl From<XmapOutput> for XmapOut {
    fn from(o: XmapOutput) -> Self {
        Self {
            langle: o.langle,
            rangle: o.rangle,
            visualmode: o.visualmode,
            prevent_change: flag(o.prevent_change),
        }
    }
}

pub struct OmapOut {
    pub cursor: Pos4,
    pub langle: Pos4,
    pub rangle: Pos4,
    pub visualmode: &'static [u8],
    /// `b"inclusive"` or `b"exclusive"`.
    pub selection: &'static [u8],
    pub prevent_change: bool,
}

impl From<OmapOutput> for OmapOut {
    fn from(o: OmapOutput) -> Self {
        Self {
            cursor: o.cursor,
            langle: o.langle,
            rangle: o.rangle,
            visualmode: o.visualmode,
            selection: o.selection,
            prevent_change: flag(o.prevent_change),
        }
    }
}

pub struct ImapOut {
    pub cursor: Pos4,
}

impl From<ImapOutput> for ImapOut {
    fn from(o: ImapOutput) -> Self {
        Self { cursor: o.cursor }
    }
}

fn current_buffer() -> NvimBuffer {
    NvimBuffer::new(api::get_current_buf())
}

/// `jieba_vim#model#nmap`.
pub fn nmap(
    state: &SharedState,
    motion: &[u8],
    cursor: Curpos5,
    count: u64,
) -> Option<NmapOut> {
    let buffer = current_buffer();
    with_wm(state, |wm| {
        wm.nmap(&buffer, motion, cursor, count)
            .ok()
            .map(NmapOut::from)
    })
    .flatten()
}

/// One `nmap` step from an explicit position, for the motion preview.
pub fn nmap_step(
    state: &SharedState,
    motion: &[u8],
    lnum: usize,
    col: usize,
) -> Option<(usize, usize)> {
    let buffer = current_buffer();
    let out = with_wm(state, |wm| {
        wm.nmap(&buffer, motion, [0, lnum, col, 0, col], 1).ok()
    })
    .flatten()?;
    let [_, lnum, col, ..] = out.cursor;
    Some((lnum, col))
}

/// `jieba_vim#model#xmap`.
pub fn xmap(
    state: &SharedState,
    visualmode: &[u8],
    motion: &[u8],
    visual_begin: Pos4,
    visual_end: Pos4,
    count: u64,
) -> Option<XmapOut> {
    let buffer = current_buffer();
    with_wm(state, |wm| {
        wm.xmap(&buffer, visualmode, motion, visual_begin, visual_end, count)
            .ok()
            .map(XmapOut::from)
    })
    .flatten()
}

/// `jieba_vim#model#omap`, including the empty-region post-check the vimscript
/// wrapper applied on top of the raw engine result.
pub fn omap(
    state: &SharedState,
    motion: &[u8],
    cursor: Curpos5,
    count: u64,
    operator: &[u8],
) -> Option<OmapOut> {
    let buffer = current_buffer();
    let mut out = with_wm(state, |wm| {
        wm.omap(&buffer, motion, cursor, count, operator)
            .ok()
            .map(OmapOut::from)
    })
    .flatten()?;

    // Selecting an empty region with an exclusive motion is an error when
    // 'cpoptions' contains "E", and must abort the operator instead of running
    // it over nothing.
    if out.langle == out.rangle
        && out.selection == b"exclusive"
        && out.visualmode != b"V"
        && !out.prevent_change
        && cpo_contains('E')
    {
        out.prevent_change = true;
    }
    Some(out)
}

/// `stridx(&cpoptions, {c}) >= 0`.
fn cpo_contains(c: char) -> bool {
    let cpo = crate::nvimrs::get_option_as::<String>("cpoptions", 0, 0)
        .unwrap_or_default();
    cpo.contains(c)
}

/// `jieba_vim#model#imap`.
pub fn imap(
    state: &SharedState,
    motion: &[u8],
    cursor: Curpos5,
) -> Option<ImapOut> {
    let buffer = current_buffer();
    with_wm(state, |wm| {
        wm.imap(&buffer, motion, cursor).ok().map(ImapOut::from)
    })
    .flatten()
}
