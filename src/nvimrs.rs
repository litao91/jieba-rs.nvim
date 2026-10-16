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

//! Native FFI bindings to Neovim's C API, matching the **0.13-dev** ABI.
//!
//! Derived from `nvim-matchup-rs/src/nvimrs.rs` (MIT, (c) litao91), extended
//! with `<expr>` keymap callbacks, buffer-local keymaps, augroups and a general
//! option setter.
//!
//! nvim-oxi 0.6.0 hardcodes Neovim v0.9/v0.10 signatures and keyset layouts.
//! On 0.13-dev these break:
//!   * `nvim_call_function` gained a leading `uint64_t channel_id`
//!     (nvim commit 8d51e07fa1) -> oxi's 4-arg binding misaligns -> SIGSEGV.
//!   * `nvim_echo` now returns `Object` (was void) -> oxi never passes the
//!     hidden sret pointer -> broken.
//!   * `nvim_create_autocmd` / `nvim_create_user_command` / `nvim_set_keymap`
//!     `KeyDict_*` layouts drifted, so oxi's `.callback(...)` LuaRef is read at
//!     the wrong offset -> the hook registers but the callback never fires.
//!   * `nvim_set_option_value` gained an `Arena *` and an `Object` return.
//!   * `nvim_set_hl`'s `Dict(highlight)` grew fields.
//!
//! This module declares those functions with their true current signatures
//! (checked against `~/repos/neovim/build/include/api/*.h.generated.h`) and the
//! exact generated keyset layouts
//! (`~/repos/neovim/build/src/nvim/auto/keysets_defs.generated.h`), reusing
//! nvim-oxi's layout-compatible `Object`/`Array`/`Dictionary` for data.
//! Everything else (buffer/window handles, `get_lines`, extmarks, `command`,
//! `feedkeys`, ...) keeps using nvim-oxi, whose bindings for those are still
//! correct.
//!
//! # Safety contract (every `unsafe` in this module)
//!
//! All `unsafe` here is either an FFI call into Neovim's C API or a
//! `mem::zeroed` keyset, and each relies on the same invariants (restated
//! briefly at the individual sites):
//!
//! * **ABI** - each `extern "C"` declaration matches nvim 0.13's true signature
//!   (checked against the generated api headers), and every `KeyDict_*` layout
//!   is pinned by the compile-time `size_of!` / `offset_of!` asserts below, so
//!   field offsets (notably the `Union(String, LuaRef)` callbacks and the
//!   generator-added `is_set`) cannot silently drift the way oxi 0.6's
//!   hardcoded 0.9/0.10 layouts do.
//! * **String args** - passed as `CStr { data, size }` from `cstr()`, whose
//!   backing `CString` is bound to a local guard that lives until the end of
//!   the function, i.e. past the call, so `data` stays valid, NUL-terminated
//!   and `size` bytes long for the whole call.
//! * **Borrowed arrays** - `CArray::borrow` is a non-owning view of a live
//!   `nvim_oxi::Array` that outlives the call; the view has no `Drop`, so nvim's
//!   read of it cannot double-free.
//! * **Return objects** - `arena` is `null`, so nvim allocates results with
//!   `xmalloc`; the returned `Object`/`Array`/`String` is layout-compatible with
//!   oxi's and frees that allocation on `Drop` (callers must not free it again).
//! * **Error out-param** - `&mut err` points to a valid `CError` (etype -1 =
//!   none) that only nvim writes. On error nvim allocates `err.msg`; we leak
//!   that rare, small string instead of calling `api_clear_error` (a leak, not
//!   UB).
//! * **Channel / threading** - `LUA_INTERNAL_CALL` marks the call in-process,
//!   which is correct because this code only runs on nvim's main thread inside a
//!   Lua / autocmd / keymap / user-command callback (nvim's API is
//!   main-thread-only, and the plugin never spawns threads).
//! * **`mem::zeroed` keysets** - every `KeyDict_*` is `#[repr(C)]` and all-zero
//!   is a valid value: `is_set = 0` (so nvim reads no optional field), bools
//!   false, ints 0, `CStr { null, 0 }` (nvim's empty `String`), and an `Object`
//!   type-tag of 0 == `kObjectTypeNil`. Optional fields are only read when we
//!   explicitly set their `is_set` bit.

use std::ffi::{CString, c_char, c_void};
use std::ptr;

use nvim_oxi::conversion::FromObject;
use nvim_oxi::{Array, Dictionary, Object};

/// `LUA_INTERNAL_CALL` = `(1<<63) + 1` (defs.h: INTERNAL_CALL_MASK, VIML/LUA).
const LUA_INTERNAL_CALL: u64 = (1u64 << 63) + 1;

// --- repr(C) primitives matching Neovim's layout ---------------------------

/// Neovim `String` = `{ char *data; size_t size; }`.
#[repr(C)]
#[derive(Clone, Copy)]
struct CStr {
    data: *const c_char,
    size: usize,
}

/// Neovim `Error` = `{ ErrorType type; char *msg; }`, `kErrorTypeNone == -1`.
#[repr(C)]
#[derive(Clone, Copy)]
struct CError {
    etype: i32,
    msg: *mut c_char,
}

impl CError {
    const fn new() -> Self {
        CError {
            etype: -1,
            msg: ptr::null_mut(),
        }
    }
    fn is_err(&self) -> bool {
        self.etype != -1
    }
}

/// Neovim `Array` (kvec) = `{ size_t size; size_t capacity; Object *items; }`.
/// Borrowed view: no `Drop`, so the owning `nvim_oxi::Array` stays in charge.
#[repr(C)]
#[derive(Clone, Copy)]
struct CArray {
    size: usize,
    capacity: usize,
    items: *const Object,
}

impl CArray {
    fn borrow(a: &Array) -> CArray {
        CArray {
            size: a.len(),
            capacity: a.len(),
            items: a.as_ptr(),
        }
    }
}

/// `KeyDict_create_augroup` (keysets_defs.h:158-161).
#[repr(C)]
struct KeyDictCreateAugroup {
    is_set: u64,
    clear: bool,
}
const OPTIDX_AUGROUP_CLEAR: u64 = 1;
const _: () = assert!(std::mem::size_of::<KeyDictCreateAugroup>() == 16);

/// `KeyDict_create_autocmd` (keysets_defs.h:277-288), field order = memory
/// order. Bit indices from keysets_defs.generated.h:95-103.
#[repr(C)]
struct KeyDictCreateAutocmd {
    is_set: u64,
    buffer: i32,      // Buffer (deprecated)
    buf: i32,         // Buffer
    callback: Object, // Union(String, LuaRef)
    command: CStr,
    desc: CStr,
    group: Object, // Union(Integer, String)
    nested: bool,
    once: bool,
    pattern: Object, // Union(String, ArrayOf(String))
}
const OPTIDX_AUTOCMD_BUF: u64 = 1;
const OPTIDX_AUTOCMD_ONCE: u64 = 3;
const OPTIDX_AUTOCMD_GROUP: u64 = 4;
const OPTIDX_AUTOCMD_PATTERN: u64 = 8;
const OPTIDX_AUTOCMD_CALLBACK: u64 = 9;

impl KeyDictCreateAutocmd {
    fn zeroed() -> Self {
        // SAFETY: every field is a plain POD / zero-valid Object (Nil).
        unsafe { std::mem::zeroed() }
    }
}

/// `KeyDict_option` (keysets_defs.h:171-180).
#[repr(C)]
struct KeyDictOption {
    is_set: u64,
    scope: CStr,
    win: i32,
    buf: i32,
    tab: i32,
    filetype: CStr,
    operation: CStr,
    dry_run: bool,
}
const OPTIDX_OPTION_BUF: u64 = 1;
const OPTIDX_OPTION_WIN: u64 = 3;

/// `KeyDict_echo_opts` (keysets_defs.h:370-382); passed zeroed (all defaults).
#[repr(C)]
struct KeyDictEchoOpts {
    is_set: u64,
    err: bool,
    verbose: bool,
    truncate: bool,
    kind: CStr,
    id: Object,
    title: CStr,
    status: CStr,
    percent: i64,
    source: CStr,
    data: CArray, // Dict = kvec(KeyValuePair); zeroed = empty, never read (is_set=0)
}

/// `KeyDict_highlight_cterm` (keysets_defs.h:222-239). Empirically 24 bytes:
/// the generated keyset carries a leading `OptionalKeys is_set` even though the
/// hand-written struct lists only the 16 bools (verified by the offset asserts
/// below + runtime link behaviour).
#[repr(C)]
struct KeyDictHighlightCterm {
    is_set: u64,
    bold: bool,
    standout: bool,
    strikethrough: bool,
    underline: bool,
    undercurl: bool,
    underdouble: bool,
    underdotted: bool,
    underdashed: bool,
    italic: bool,
    reverse: bool,
    altfont: bool,
    dim: bool,
    blink: bool,
    conceal: bool,
    overline: bool,
    nocombine: bool,
}

/// `KeyDict_highlight` (keysets_defs.h:182-220), field order = memory order.
/// `Union(Integer, String)` fields are nvim `Object`; `HLGroupID` is `Integer`.
#[repr(C)]
struct KeyDictHighlight {
    is_set: u64,
    altfont: bool,
    blink: bool,
    bold: bool,
    conceal: bool,
    dim: bool,
    italic: bool,
    nocombine: bool,
    overline: bool,
    reverse: bool,
    standout: bool,
    strikethrough: bool,
    undercurl: bool,
    underdashed: bool,
    underdotted: bool,
    underdouble: bool,
    underline: bool,
    default_: bool,
    cterm: KeyDictHighlightCterm,
    foreground: Object,
    fg: Object,
    background: Object,
    bg: Object,
    ctermfg: Object,
    ctermbg: Object,
    special: Object,
    sp: Object,
    link: i64, // HLGroupID
    link_global: i64,
    fallback: bool,
    blend: i64,
    fg_indexed: bool,
    bg_indexed: bool,
    force: bool,
    update: bool,
    url: CStr,
    font: CStr,
}
const OPTIDX_HL_LINK: u64 = 8;
const OPTIDX_HL_DEFAULT: u64 = 16;

// Compile-time layout guards: if the nvim keyset ever drifts (or Object/CStr
// are not the assumed size), these fail the build instead of silently misreading
// the struct at runtime (the failure mode that broke oxi's create_autocmd).
const _: () = assert!(std::mem::size_of::<Object>() == 32);
const _: () = assert!(std::mem::size_of::<CStr>() == 16);
const _: () = assert!(std::mem::size_of::<KeyDictHighlightCterm>() == 24);
const _: () = assert!(std::mem::size_of::<KeyDictHighlight>() == 384);
const _: () = assert!(std::mem::offset_of!(KeyDictHighlight, default_) == 24);
const _: () = assert!(std::mem::offset_of!(KeyDictHighlight, cterm) == 32);
const _: () = assert!(std::mem::offset_of!(KeyDictHighlight, link) == 312);
const _: () = assert!(std::mem::offset_of!(KeyDictHighlight, url) == 352);

/// `KeyDict_user_command` (keysets_defs.h:99-113), field order = memory order.
#[repr(C)]
struct KeyDictUserCommand {
    is_set: u64,
    addr: Object,
    bang: bool,
    bar: bool,
    complete: Object,
    count: Object,
    desc: Object,
    force: bool,
    keepscript: bool,
    nargs: Object,
    preview: Object,
    range: Object,
    register_: bool,
}
const OPTIDX_UCMD_DESC: u64 = 4;
const OPTIDX_UCMD_FORCE: u64 = 6;
const _: () = assert!(std::mem::size_of::<KeyDictUserCommand>() == 256);
const _: () = assert!(std::mem::offset_of!(KeyDictUserCommand, desc) == 112);
const _: () = assert!(std::mem::offset_of!(KeyDictUserCommand, force) == 144);

/// `KeyDict_keymap` (keysets_defs.h:84-95), field order = memory order.
/// NOTE: `callback` is a bare `LuaRef` (= `int`, types_defs.h:28), *not* a
/// `Union(String, LuaRef)` Object like the autocmd/user-command callbacks - so
/// it is 4 bytes at offset 16 and the six bools before it leave 2 pad bytes;
/// `_pad` makes the following `CStr`'s 8-byte alignment explicit rather than
/// implicit.
#[repr(C)]
struct KeyDictKeymap {
    is_set: u64,
    noremap: bool,
    nowait: bool,
    silent: bool,
    script: bool,
    expr: bool,
    unique: bool,
    callback: i32, // LuaRef
    _pad: i32,
    desc: CStr,
    replace_keycodes: bool,
}
// Bit indices from keysets_defs.generated.h:195-203.
const OPTIDX_KEYMAP_DESC: u64 = 1;
const OPTIDX_KEYMAP_EXPR: u64 = 2;
const OPTIDX_KEYMAP_SILENT: u64 = 4;
const OPTIDX_KEYMAP_NOREMAP: u64 = 7;
const OPTIDX_KEYMAP_CALLBACK: u64 = 8;
const OPTIDX_KEYMAP_REPLACE_KEYCODES: u64 = 9;
const _: () = assert!(std::mem::size_of::<KeyDictKeymap>() == 48);
const _: () = assert!(std::mem::offset_of!(KeyDictKeymap, callback) == 16);
const _: () = assert!(std::mem::offset_of!(KeyDictKeymap, desc) == 24);

// SAFETY: these declarations mirror nvim 0.13's exported C ABI (checked against
// the generated api headers, with every KeyDict_* layout pinned by the asserts
// above). Calling them is `unsafe`; each wrapper below upholds the module-level
// Safety contract - CStr/array args backed by locals that outlive the call, a
// valid `&mut` error out-param, null arena (returned objects own nvim's alloc),
// main-thread only, and LUA_INTERNAL_CALL where a channel id is taken.
unsafe extern "C" {
    fn nvim_call_function(
        channel_id: u64,
        name: CStr,
        args: CArray,
        arena: *mut c_void,
        err: *mut CError,
    ) -> Object;
    fn nvim_echo(
        chunks: CArray,
        history: bool,
        opts: *const KeyDictEchoOpts,
        err: *mut CError,
    ) -> Object;
    fn nvim_create_augroup(
        channel_id: u64,
        name: CStr,
        opts: *const KeyDictCreateAugroup,
        err: *mut CError,
    ) -> i64;
    fn nvim_create_autocmd(
        channel_id: u64,
        event: Object,
        opts: *const KeyDictCreateAutocmd,
        arena: *mut c_void,
        err: *mut CError,
    ) -> i64;
    fn nvim_get_hl_id_by_name(name: CStr, err: *mut CError) -> i64;
    fn nvim_set_hl(
        channel_id: u64,
        ns_id: i64,
        name: CStr,
        val: *const KeyDictHighlight,
        err: *mut CError,
    );
    fn nvim_create_user_command(
        channel_id: u64,
        name: CStr,
        cmd: Object,
        opts: *const KeyDictUserCommand,
        err: *mut CError,
    );
    fn nvim_set_keymap(
        channel_id: u64,
        mode: CStr,
        lhs: CStr,
        rhs: CStr,
        opts: *const KeyDictKeymap,
        err: *mut CError,
    );
    fn nvim_buf_set_keymap(
        channel_id: u64,
        buf: i32,
        mode: CStr,
        lhs: CStr,
        rhs: CStr,
        opts: *const KeyDictKeymap,
        err: *mut CError,
    );
    fn nvim_get_option_value(
        name: CStr,
        opts: *const KeyDictOption,
        err: *mut CError,
    ) -> Object;
    // 0.13 sig: (channel_id, name, value, opts, arena, err) -> Object. oxi 0.6
    // declares this against v0.10 (no `arena`, `void` return) -> ABI-broken here.
    fn nvim_set_option_value(
        channel_id: u64,
        name: CStr,
        value: Object,
        opts: *const KeyDictOption,
        arena: *mut c_void,
        err: *mut CError,
    ) -> Object;
    fn nvim_get_vvar(
        name: CStr,
        arena: *mut c_void,
        err: *mut CError,
    ) -> Object;
    fn nvim_command(cmd: CStr, err: *mut CError);
}

// --- safe wrappers ---------------------------------------------------------

fn cstr(s: &str) -> Option<(CString, CStr)> {
    let c = CString::new(s).ok()?;
    let cs = CStr {
        data: c.as_ptr(),
        size: s.len(),
    };
    Some((c, cs))
}

/// Like [`cstr`], but for byte strings that are not valid UTF-8.
///
/// Neovim's own `String` is a length-tagged byte buffer, not necessarily UTF-8,
/// while oxi's `Object` can only carry a Rust `String`. A few vimscript values
/// are genuinely non-UTF-8 - a blockwise `visualmode()` is the single byte
/// `\x16` - so the Ex commands that have to echo them back are built from bytes
/// here instead of going through `api::command`.
fn cstr_bytes(s: &[u8]) -> Option<(Vec<u8>, CStr)> {
    // An interior NUL would truncate the command; nvim's own strings never
    // contain one.
    if s.contains(&0) {
        return None;
    }
    let mut buf = Vec::with_capacity(s.len() + 1);
    buf.extend_from_slice(s);
    buf.push(0);
    // `buf` is returned alongside the view and never mutated again, so the
    // pointer stays valid for the whole call.
    let cs = CStr {
        data: buf.as_ptr() as *const c_char,
        size: s.len(),
    };
    Some((buf, cs))
}

/// Native `nvim_command` with a raw-byte command line - `execute {cmd}` where
/// `{cmd}` may contain bytes that are not valid UTF-8.
pub fn command_raw(cmd: &[u8]) -> bool {
    let (_guard, ccmd) = match cstr_bytes(cmd) {
        Some(x) => x,
        None => return false,
    };
    let mut err = CError::new();
    // SAFETY: module contract; `ccmd` is backed by `_guard`, which outlives the
    // call, and `err` is a valid &mut. `nvim_command` returns nothing and takes
    // no channel id or arena (vimscript.h.generated.h:14).
    unsafe { nvim_command(ccmd, &mut err) };
    !err.is_err()
}

/// The raw bytes of an nvim String object, without requiring it to be UTF-8.
pub fn object_bytes(obj: &Object) -> Option<Vec<u8>> {
    let s = nvim_oxi::String::from_object(obj.clone()).ok()?;
    Some(s.as_bytes().to_vec())
}

/// Build a `KeyDictKeymap` with `silent` set; the caller decides `noremap`,
/// `expr`, `replace_keycodes` and `callback`.
///
/// SAFETY: all-zero `KeyDictKeymap` is a valid bit pattern (`is_set = 0`, so
/// nvim reads no optional field yet; bools false; `desc` = empty CStr; the bare
/// `i32` `callback` is overwritten by the caller before the call). Layout is
/// compile-time asserted above.
fn keymap_opts_zeroed() -> KeyDictKeymap {
    let mut opts: KeyDictKeymap = unsafe { std::mem::zeroed() };
    opts.silent = true;
    opts.is_set |= 1 << OPTIDX_KEYMAP_SILENT;
    opts
}

/// Native `nvim_call_function` (correct 0.13 signature with leading
/// channel_id). `args` is borrowed for the duration of the call.
pub fn call_function(name: &str, args: &Array) -> Option<Object> {
    let (_guard, cname) = cstr(name)?;
    let cargs = CArray::borrow(args);
    let mut err = CError::new();
    // SAFETY: module contract; `cname` is backed by `_guard`, `cargs` borrows
    // the live `args` Array (non-owning view, no Drop), `err` is a valid &mut,
    // arena null so the returned Object owns nvim's allocation.
    let obj = unsafe {
        nvim_call_function(
            LUA_INTERNAL_CALL,
            cname,
            cargs,
            ptr::null_mut(),
            &mut err,
        )
    };
    if err.is_err() {
        return None;
    }
    Some(obj)
}

pub fn call_fn_as<V: FromObject>(name: &str, args: &Array) -> Option<V> {
    let obj = call_function(name, args)?;
    V::from_object(obj).ok()
}

/// `call_function(name, [])`
pub fn call_fn0_as<V: FromObject>(name: &str) -> Option<V> {
    call_fn_as(name, &Array::new())
}

/// Native `nvim_echo` with a single plain-text chunk (history=true).
pub fn echo(text: &str) {
    let chunks =
        Array::from_iter([Object::from(Array::from_iter([Object::from(
            text,
        )]))]);
    let cchunks = CArray::borrow(&chunks);
    // SAFETY: all-zero `KeyDictEchoOpts` is a valid value (is_set = 0, so nvim
    // reads no optional field); see the module Safety contract.
    let opts: KeyDictEchoOpts = unsafe { std::mem::zeroed() };
    let mut err = CError::new();
    // SAFETY: module contract; `cchunks` borrows the live `chunks` local, opts
    // and err are valid pointers, and the returned Object (`_ret`) owns and
    // frees nvim's allocation when it drops at the end of this function.
    let _ret = unsafe { nvim_echo(cchunks, true, &opts, &mut err) };
    // _ret (Object) drops here, freeing nvim's allocation.
}

/// Native option read. `buf`/`win` = 0 means global scope.
pub fn get_option_as<V: FromObject>(
    name: &str,
    buf: i32,
    win: i32,
) -> Option<V> {
    let (_guard, cname) = cstr(name)?;
    // SAFETY: all-zero `KeyDictOption` is valid (is_set = 0); the optional
    // buf/win fields are set below only alongside their is_set bits. Module
    // contract.
    let mut opts: KeyDictOption = unsafe { std::mem::zeroed() };
    if buf != 0 {
        opts.buf = buf;
        opts.is_set |= 1 << OPTIDX_OPTION_BUF;
    }
    if win != 0 {
        opts.win = win;
        opts.is_set |= 1 << OPTIDX_OPTION_WIN;
    }
    let mut err = CError::new();
    // SAFETY: module contract; `cname` is backed by `_guard` (outlives the
    // call), `opts`/`err` are valid pointers, returned Object owns nvim's alloc.
    let obj = unsafe { nvim_get_option_value(cname, &opts, &mut err) };
    if err.is_err() {
        return None;
    }
    V::from_object(obj).ok()
}

/// Native option write (`nvim_set_option_value`). `buf`/`win` = 0 means the
/// global value; `value` is moved in and nvim copies it synchronously.
///
/// Hand-rolled because oxi 0.6's binding targets nvim v0.10 (missing the
/// `arena` param and the `Object` return) and is ABI-broken on 0.13.
pub fn set_option_value(name: &str, value: Object, buf: i32, win: i32) -> bool {
    let (_ng, cname) = match cstr(name) {
        Some(x) => x,
        None => return false,
    };
    // nvim copies `value` synchronously and does not adopt it, so ownership
    // stays with `value` (freed once at end of scope) and C gets a bitwise
    // copy. The copy moved into the `extern "C"` call is forgotten (such args
    // are not dropped), so the object is freed exactly once.
    let vcopy = unsafe { ptr::read(&value) };
    // SAFETY: all-zero `KeyDictOption` is valid (is_set = 0); buf/win set with
    // their is_set bits. Rest per the module contract.
    let mut opts: KeyDictOption = unsafe { std::mem::zeroed() };
    if buf != 0 {
        opts.buf = buf;
        opts.is_set |= 1 << OPTIDX_OPTION_BUF;
    }
    if win != 0 {
        opts.win = win;
        opts.is_set |= 1 << OPTIDX_OPTION_WIN;
    }
    let mut err = CError::new();
    // SAFETY: module contract; `cname` is backed by `_ng`, arena is null so any
    // nvim allocation uses xmalloc (the returned `Object` owns and frees it).
    let ret = unsafe {
        nvim_set_option_value(
            LUA_INTERNAL_CALL,
            cname,
            vcopy,
            &opts,
            ptr::null_mut(),
            &mut err,
        )
    };
    drop(ret);
    !err.is_err()
}

/// Native `nvim_get_vvar` (v: scope, e.g. count/operator/register).
pub fn get_vvar_as<V: FromObject>(name: &str) -> Option<V> {
    let (_g, cname) = cstr(name)?;
    let mut err = CError::new();
    // SAFETY: module contract; `cname` is backed by `_g`, `err` is a valid &mut,
    // arena null so the returned Object owns nvim's allocation.
    let obj = unsafe { nvim_get_vvar(cname, ptr::null_mut(), &mut err) };
    if err.is_err() {
        None
    } else {
        V::from_object(obj).ok()
    }
}

/// Native `nvim_create_augroup`.
pub fn create_augroup(name: &str, clear: bool) -> Option<i64> {
    let (_g, cname) = cstr(name)?;
    // SAFETY: all-zero is valid for this POD keyset; `clear` is set with its
    // is_set bit. Module contract.
    let mut opts: KeyDictCreateAugroup = unsafe { std::mem::zeroed() };
    opts.clear = clear;
    opts.is_set |= 1 << OPTIDX_AUGROUP_CLEAR;
    let mut err = CError::new();
    // SAFETY: module contract; `cname` is backed by `_g`, `opts` layout is
    // compile-time asserted, `err` is a valid &mut.
    let id = unsafe {
        nvim_create_augroup(LUA_INTERNAL_CALL, cname, &opts, &mut err)
    };
    if err.is_err() || id <= 0 {
        None
    } else {
        Some(id)
    }
}

/// Register an autocmd whose handler is a native Rust callback (a real LuaRef
/// built by nvim-oxi's registry, packed into a correctly-laid-out
/// `KeyDict_create_autocmd`). `f` returns whether to delete the autocmd.
/// `buf` non-zero makes it buffer-local; `pattern` may be `""` in that case.
pub fn create_autocmd_cb<F>(
    events: &[&str],
    group: i64,
    pattern: &str,
    buf: i32,
    once: bool,
    f: F,
) -> bool
where
    F: Fn(nvim_oxi::api::types::AutocmdCallbackArgs) -> bool + 'static,
{
    use nvim_oxi::Function;
    let func: Function<nvim_oxi::api::types::AutocmdCallbackArgs, bool> =
        Function::from_fn(move |args| -> nvim_oxi::Result<bool> {
            Ok(f(args))
        });
    let cb_obj = Object::from(func); // tagged LuaRef object

    let event_arr = Array::from_iter(events.iter().map(|e| Object::from(*e)));
    let event = Object::from(event_arr);

    let mut opts = KeyDictCreateAutocmd::zeroed();
    opts.callback = cb_obj;
    opts.is_set |= 1 << OPTIDX_AUTOCMD_CALLBACK;
    opts.group = Object::from(group);
    opts.is_set |= 1 << OPTIDX_AUTOCMD_GROUP;
    if buf != 0 {
        opts.buf = buf;
        opts.is_set |= 1 << OPTIDX_AUTOCMD_BUF;
    } else {
        opts.pattern = Object::from(pattern.to_string());
        opts.is_set |= 1 << OPTIDX_AUTOCMD_PATTERN;
    }
    if once {
        opts.once = true;
        opts.is_set |= 1 << OPTIDX_AUTOCMD_ONCE;
    }

    let mut err = CError::new();
    // SAFETY: module contract; `event`/`opts.pattern`/`opts.group` are owned
    // Objects valid for the call, `opts` is a KeyDictCreateAutocmd whose layout
    // is compile-time asserted, its `callback` is a LuaRef Object that nvim
    // adopts (oxi's Object does not unref a LuaRef on Drop, so no double-free),
    // `err` is a valid &mut, and arena is null.
    let id = unsafe {
        nvim_create_autocmd(
            LUA_INTERNAL_CALL,
            event,
            &opts,
            ptr::null_mut(),
            &mut err,
        )
    };
    !err.is_err() && id > 0
}

/// Link highlight group `name` to `target` in the global namespace.
/// `default = true` mirrors `hi def link` (an existing user definition wins);
/// `default = false` mirrors plain `hi link`. `link` takes an HLGroupID, so
/// `target` is resolved by name first.
pub fn set_hl_link(name: &str, target: &str, default: bool) {
    let (_tguard, ctarget) = match cstr(target) {
        Some(x) => x,
        None => return,
    };
    let mut err = CError::new();
    // SAFETY: module contract; `ctarget` is backed by `_tguard` (outlives the
    // call) and `err` is a valid &mut. Returns an HLGroupID (i64).
    let id = unsafe { nvim_get_hl_id_by_name(ctarget, &mut err) };
    if err.is_err() {
        return;
    }
    let (_nguard, cname) = match cstr(name) {
        Some(x) => x,
        None => return,
    };
    // SAFETY: all-zero KeyDictHighlight is valid - is_set = 0, bools false, the
    // nested KeyDictHighlightCterm is itself all-zero (its own is_set = 0), and
    // the String/Object/Integer fields are zero-valid (Nil/empty). Layout is
    // compile-time asserted; only `link`/`default_` are set below, each with its
    // is_set bit.
    let mut val: KeyDictHighlight = unsafe { std::mem::zeroed() };
    val.link = id;
    val.is_set |= 1 << OPTIDX_HL_LINK;
    if default {
        val.default_ = true;
        val.is_set |= 1 << OPTIDX_HL_DEFAULT;
    }
    let mut err = CError::new();
    // SAFETY: module contract; ns_id 0 = global namespace, `cname` is backed by
    // `_nguard`, `val` is a valid asserted-layout KeyDictHighlight, `err` is a
    // valid &mut.
    unsafe { nvim_set_hl(LUA_INTERNAL_CALL, 0, cname, &val, &mut err) };
}

/// Define a user command whose handler is a native Rust callback (a LuaRef
/// packed as the `Union(String, LuaRef)` cmd), via the correctly-laid-out
/// `KeyDict_user_command`. `force = true` mirrors `command!`. The callback
/// receives the command-args table (ignored by our no-arg commands).
pub fn create_user_command_cb<F>(name: &str, desc: &str, f: F) -> bool
where
    F: Fn(Dictionary) + 'static,
{
    use nvim_oxi::Function;
    let func: Function<Dictionary, ()> =
        Function::from_fn(move |args| -> nvim_oxi::Result<()> {
            f(args);
            Ok(())
        });
    let cmd = Object::from(func); // tagged LuaRef object

    let (_ng, cname) = match cstr(name) {
        Some(x) => x,
        None => return false,
    };
    // SAFETY: all-zero KeyDictUserCommand is valid (is_set = 0, Object fields
    // Nil, bools false); layout is compile-time asserted. Module contract.
    let mut opts: KeyDictUserCommand = unsafe { std::mem::zeroed() };
    opts.force = true;
    opts.is_set |= 1 << OPTIDX_UCMD_FORCE;
    opts.desc = Object::from(desc.to_string());
    opts.is_set |= 1 << OPTIDX_UCMD_DESC;
    let mut err = CError::new();
    // SAFETY: module contract; `cname` is backed by `_ng`, `cmd` is a LuaRef
    // Object nvim adopts (oxi's Object does not unref a LuaRef on Drop, so no
    // double-free), `opts` layout is asserted, `err` is a valid &mut.
    unsafe {
        nvim_create_user_command(LUA_INTERNAL_CALL, cname, cmd, &opts, &mut err)
    };
    !err.is_err()
}

/// Shared body of the keymap registration helpers. `rhs` is only used when
/// `callback` is `None`.
///
/// nvim takes ownership of the LuaRef (mapping.c sets `opts->callback =
/// LUA_NOREF`) and oxi's `Function` has no `Drop`, so there is no
/// double-unref; the ref lives until the mapping is deleted.
#[allow(clippy::too_many_arguments)]
fn set_keymap_inner(
    buf: i32,
    mode: &str,
    lhs: &str,
    rhs: &str,
    desc: &str,
    luaref: Option<i32>,
    expr: bool,
    noremap: bool,
) -> bool {
    let (_mg, cmode) = match cstr(mode) {
        Some(x) => x,
        None => return false,
    };
    let (_lg, clhs) = match cstr(lhs) {
        Some(x) => x,
        None => return false,
    };
    let (_rg, crhs) = match cstr(rhs) {
        Some(x) => x,
        None => return false,
    };
    let (_dg, cdesc) = match cstr(desc) {
        Some(x) => x,
        None => return false,
    };

    let mut opts = keymap_opts_zeroed();
    // `noremap` is per-mapping, not forced: a default keymap whose rhs is a
    // `<Plug>` name must stay remappable or the `<Plug>` never resolves.
    opts.noremap = noremap;
    if noremap {
        opts.is_set |= 1 << OPTIDX_KEYMAP_NOREMAP;
    }
    if let Some(r) = luaref {
        opts.callback = r;
        opts.is_set |= 1 << OPTIDX_KEYMAP_CALLBACK;
    }
    if expr {
        // `replace_keycodes` is only honoured together with `expr`
        // (mapping.c:2810) and must be true for `<Cmd>`/`<CR>` markup in the
        // returned string to be translated into real keycodes.
        opts.expr = true;
        opts.is_set |= 1 << OPTIDX_KEYMAP_EXPR;
        opts.replace_keycodes = true;
        opts.is_set |= 1 << OPTIDX_KEYMAP_REPLACE_KEYCODES;
    }
    opts.desc = cdesc;
    opts.is_set |= 1 << OPTIDX_KEYMAP_DESC;

    let mut err = CError::new();
    // SAFETY: module contract; `cmode`/`clhs`/`crhs`/`cdesc` are backed by their
    // guards (all outlive the call), `opts.callback` is a LuaRef nvim adopts,
    // `opts` layout is compile-time asserted, and `err` is a valid &mut.
    unsafe {
        if buf == 0 {
            nvim_set_keymap(
                LUA_INTERNAL_CALL,
                cmode,
                clhs,
                crhs,
                &opts,
                &mut err,
            )
        } else {
            nvim_buf_set_keymap(
                LUA_INTERNAL_CALL,
                buf,
                cmode,
                clhs,
                crhs,
                &opts,
                &mut err,
            )
        }
    };
    !err.is_err()
}

/// Define a keymap whose handler is a native Rust callback taking no arguments
/// and returning nothing - the equivalent of `nnoremap <silent> lhs <Lua fn>`.
/// Keymap callbacks are invoked with no arguments (mapping.c: `Array args =
/// ARRAY_DICT_INIT`).
///
/// Must NOT be used for operator-pending maps that have to cancel the pending
/// operator: a non-`expr` Lua rhs is stored as `"<K_LUA><id><CR>"`, and
/// `nv_colon` sets `motion_type = kMTCharWise` + runs `do_pending_operator`
/// before any keys the callback feeds are consumed. Use `set_keymap_expr_cb`.
pub fn set_keymap_cb<F>(
    buf: i32,
    mode: &str,
    lhs: &str,
    desc: &str,
    f: F,
) -> bool
where
    F: Fn() + 'static,
{
    use nvim_oxi::Function;
    let func: Function<(), ()> =
        Function::from_fn(move |()| -> nvim_oxi::Result<()> {
            f();
            Ok(())
        });
    // oxi's LuaRef is c_int (i32), matching nvim's `typedef int LuaRef`.
    set_keymap_inner(
        buf,
        mode,
        lhs,
        "",
        desc,
        Some(func.lua_ref()),
        false,
        true,
    )
}

/// Define an `<expr>` keymap whose handler is a native Rust callback returning
/// the replacement key string - the equivalent of `nnoremap <expr> ... f()`.
/// `replace_keycodes` is on, so the returned string may contain `<Cmd>...<CR>`
/// markup.
///
/// nvim holds `expr_map_lock` while the callback runs (mapping.c:1679): `:normal`
/// refuses with E943 and `u_save` refuses. The callback must therefore be
/// strictly read-only - compute, stash, return.
pub fn set_keymap_expr_cb<F>(
    buf: i32,
    mode: &str,
    lhs: &str,
    desc: &str,
    f: F,
) -> bool
where
    F: Fn() -> String + 'static,
{
    use nvim_oxi::Function;
    let func: Function<(), String> =
        Function::from_fn(move |()| -> nvim_oxi::Result<String> { Ok(f()) });
    set_keymap_inner(buf, mode, lhs, "", desc, Some(func.lua_ref()), true, true)
}

/// Define a keymap with a literal string rhs (`noremap` + `silent`), the
/// equivalent of `nnoremap <silent> lhs rhs`.
pub fn set_keymap_str(
    buf: i32,
    mode: &str,
    lhs: &str,
    rhs: &str,
    desc: &str,
) -> bool {
    set_keymap_inner(buf, mode, lhs, rhs, desc, None, false, true)
}

/// Define a **remappable** keymap with a literal string rhs, the equivalent of
/// `nmap lhs rhs`. This is what the default keymaps need: their rhs is a
/// `<Plug>` name, and `noremap` would stop it resolving.
pub fn set_keymap_remap(
    buf: i32,
    mode: &str,
    lhs: &str,
    rhs: &str,
    desc: &str,
) -> bool {
    set_keymap_inner(buf, mode, lhs, rhs, desc, None, false, false)
}
