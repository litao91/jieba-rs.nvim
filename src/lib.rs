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

//! jieba-rs.nvim: Chinese word motions and text objects for Neovim.
//!
//! Lua module surface (`require("jieba_rs")` loads `lua/jieba_rs.so`):
//!   setup(opts)                 - build the engine, register maps/commands/autocmds
//!   nmap/xmap/omap/imap(...)    - raw model calls (tests, benchmarks, interop)
//!   preview_nmap(...)           - the positions the preview would highlight
//!   jieba_preview_{motion}()    - highlight where that motion would land
//!   jieba_preview_cancel()      - clear the preview highlights
//!   update_isk(isk)             - re-read 'iskeyword' into the tokenizer
//!
//! All plugin logic lives in Rust. The only Lua in this repository is
//! `lua/jieba_rs/build.lua`, which shells out to `make install`.
//!
//! # Modules
//!
//! ```text
//! nvimrs.rs   native nvim C-API FFI for the parts of the 0.13 ABI that
//!             nvim-oxi 0.6 gets wrong (call_function, echo, autocmd/keymap/
//!             user-command callbacks, set_option_value, set_hl)
//! state.rs    Opts, State, and the pending-action records the <expr> callbacks
//!             stash for their <Cmd> handlers
//! jieba.rs    the segmenter (OnceLock, so lazy and eager share one type)
//! buffer.rs   BufferLike over nvim_buf_get_lines, with a per-motion snapshot
//! pos.rs      getcurpos/getpos/setpos/cursor and UTF-8-safe column clamping
//! model.rs    engine calls + outputs (port of plugin/jieba_vim/model.vim)
//! nmap.rs     normal mode                     (port of mapping#nmap)
//! xmap.rs     visual mode                     (port of mapping#xmap)
//! omap.rs     operator-pending playback       (port of mapping#omap_playback*)
//! opfunc.rs   experimental g@/operatorfunc    (port of mapping#omap_visual_expr)
//! imap.rs     insert-mode C-w and arrows      (port of mapping#imap_expr)
//! preview.rs  motion preview on extmarks      (port of mapping#preview)
//! keymap.rs   <Plug> maps, commands, autocmds (port of plugin/jieba_vim.vim)
//! util.rs     the vimscript primitives the above are built on
//! ```

pub mod buffer;
pub mod imap;
pub mod jieba;
pub mod keymap;
pub mod model;
pub mod nmap;
pub mod nvimrs;
pub mod omap;
pub mod opfunc;
pub mod pos;
pub mod preview;
pub mod state;
pub mod util;
pub mod xmap;

use std::rc::Rc;

use nvim_oxi::{Array, Dictionary, Function, Object, Result, api};

use state::{Opts, SharedState, State};

/// Log panics to a file: the default hook's stderr output is easily lost inside
/// nvim, and a panic in a Lua callback aborts the process.
fn install_panic_hook() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/jieba_rs_panic.log")
            {
                let _ = writeln!(f, "{info}");
            }
        }));
    });
}

/// Run a callback body, converting panics into `R::default()` plus an error
/// message instead of aborting nvim. Every Rust callback nvim can invoke goes
/// through this.
pub(crate) fn guard<R: Default>(name: &str, f: impl FnOnce() -> R) -> R {
    // SAFETY: single-threaded use; on panic the unwind drops all RefCell guards
    // normally, so no borrow state is poisoned.
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(e) => {
            let msg = e
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "unknown panic".to_string());
            api::err_writeln(&format!("jieba_rs: panic in {name}: {msg}"));
            R::default()
        }
    }
}

fn pos_array<const N: usize>(p: [usize; N]) -> Object {
    Object::from(Array::from_iter(
        p.into_iter().map(|n| Object::from(n as i64)),
    ))
}

fn array_to_usize<const N: usize>(arr: Array) -> Option<[usize; N]> {
    if arr.len() < N {
        return None;
    }
    let mut out = [0usize; N];
    for (slot, item) in out.iter_mut().zip(arr.iter()) {
        *slot = usize::try_from(i64::try_from(item.clone()).ok()?).ok()?;
    }
    Some(out)
}

/// Read a motion argument that may be given either as its name (`"w"`, `"ge"`)
/// or as a raw termcode string (the insert-mode arrow keys).
fn motion_bytes(motion: &str) -> Vec<u8> {
    if motion.bytes().any(|b| b >= 0x80) || motion.starts_with('<') {
        util::termcodes_bytes(motion.as_bytes())
    } else {
        motion.as_bytes().to_vec()
    }
}

#[nvim_oxi::plugin]
fn jieba_rs() -> Result<Dictionary> {
    install_panic_hook();
    let state: SharedState = Rc::new(State::new());

    let st = Rc::clone(&state);
    let setup: Function<(Option<Dictionary>,), ()> = Function::from_fn(
        move |(opts,): (Option<Dictionary>,)| -> Result<()> {
            install_panic_hook();
            guard("setup", || {
                st.set_opts(Opts::from_opts(opts.as_ref()));
                // Report a bad user dictionary once, here, rather than from
                // inside a keymap callback later.
                if let Some(err) = st.build_word_motion() {
                    nvimrs::echo(&err);
                    return;
                }
                keymap::setup_plugs(&st);
                let gopts = st.gopts();
                // With no filetype filter the default keymaps are global; with
                // one, setup_autocmds installs them buffer-locally instead.
                if gopts.filetypes.is_empty() {
                    keymap::default_keymaps(&st, 0);
                }
            });
            Ok(())
        },
    );

    let st = Rc::clone(&state);
    let update_isk: Function<(String,), ()> =
        Function::from_fn(move |(isk,): (String,)| -> Result<()> {
            guard("update_isk", || {
                if let Some(err) = st.update_isk(&isk) {
                    nvimrs::echo(&err);
                }
            });
            Ok(())
        });

    let st = Rc::clone(&state);
    let nmap: Function<(String, Array, i64), Object> = Function::from_fn(
        move |(motion, cursor, count): (String, Array, i64)| -> Result<Object> {
            Ok(guard("nmap", || -> Object {
                let Some(cursor) = array_to_usize::<5>(cursor) else {
                    return Object::nil();
                };
                let Some(out) = model::nmap(
                    &st,
                    &motion_bytes(&motion),
                    cursor,
                    count.max(1) as u64,
                ) else {
                    return Object::nil();
                };
                Object::from(Dictionary::from_iter([
                    ("cursor", pos_array(out.cursor)),
                    ("prevent_change", Object::from(out.prevent_change)),
                ]))
            }))
        },
    );

    let st = Rc::clone(&state);
    let omap: Function<(String, Array, i64, String), Object> =
        Function::from_fn(
            move |(motion, cursor, count, operator): (
                String,
                Array,
                i64,
                String,
            )|
                  -> Result<Object> {
                Ok(guard("omap", || -> Object {
                    let Some(cursor) = array_to_usize::<5>(cursor) else {
                        return Object::nil();
                    };
                    let Some(out) = model::omap(
                        &st,
                        &motion_bytes(&motion),
                        cursor,
                        count.max(1) as u64,
                        operator.as_bytes(),
                    ) else {
                        return Object::nil();
                    };
                    Object::from(Dictionary::from_iter([
                        ("cursor", pos_array(out.cursor)),
                        ("langle", pos_array(out.langle)),
                        ("rangle", pos_array(out.rangle)),
                        (
                            "visualmode",
                            Object::from(nvim_oxi::String::from_bytes(
                                out.visualmode,
                            )),
                        ),
                        (
                            "selection",
                            Object::from(nvim_oxi::String::from_bytes(
                                out.selection,
                            )),
                        ),
                        ("prevent_change", Object::from(out.prevent_change)),
                    ]))
                }))
            },
        );

    let st = Rc::clone(&state);
    let xmap: Function<(String, String, Array, Array, i64), Object> =
        Function::from_fn(
            move |(visualmode, motion, begin, end, count): (
                String,
                String,
                Array,
                Array,
                i64,
            )|
                  -> Result<Object> {
                Ok(guard("xmap", || -> Object {
                    let (Some(begin), Some(end)) =
                        (array_to_usize::<4>(begin), array_to_usize::<4>(end))
                    else {
                        return Object::nil();
                    };
                    let Some(out) = model::xmap(
                        &st,
                        util::termcodes_bytes(visualmode.as_bytes()).as_slice(),
                        &motion_bytes(&motion),
                        begin,
                        end,
                        count.max(1) as u64,
                    ) else {
                        return Object::nil();
                    };
                    Object::from(Dictionary::from_iter([
                        ("langle", pos_array(out.langle)),
                        ("rangle", pos_array(out.rangle)),
                        (
                            "visualmode",
                            Object::from(nvim_oxi::String::from_bytes(
                                out.visualmode,
                            )),
                        ),
                        ("prevent_change", Object::from(out.prevent_change)),
                    ]))
                }))
            },
        );

    let st = Rc::clone(&state);
    let imap: Function<(String, Array), Object> = Function::from_fn(
        move |(motion, cursor): (String, Array)| -> Result<Object> {
            Ok(guard("imap", || -> Object {
                let Some(cursor) = array_to_usize::<5>(cursor) else {
                    return Object::nil();
                };
                let Some(out) =
                    model::imap(&st, &motion_bytes(&motion), cursor)
                else {
                    return Object::nil();
                };
                Object::from(Dictionary::from_iter([(
                    "cursor",
                    pos_array(out.cursor),
                )]))
            }))
        },
    );

    let st = Rc::clone(&state);
    let preview_nmap: Function<(String, Array, i64), Object> =
        Function::from_fn(move |(motion, cursor, limit): (String, Array, i64)| -> Result<Object> {
            Ok(guard("preview_nmap", || -> Object {
                let Some(cursor) = array_to_usize::<5>(cursor) else {
                    return Object::nil();
                };
                let Some(out) =
                    preview::positions_at(&st, &motion_bytes(&motion), cursor, limit)
                else {
                    return Object::nil();
                };
                Object::from(Array::from_iter(out.into_iter().map(|(lnum, col)| {
                    Object::from(Array::from_iter([
                        Object::from(lnum as i64),
                        Object::from(col as i64),
                    ]))
                })))
            }))
        });

    let st = Rc::clone(&state);
    let default_keymaps: Function<(Option<i64>,), ()> =
        Function::from_fn(move |(buf,): (Option<i64>,)| -> Result<()> {
            guard("default_keymaps", || {
                keymap::default_keymaps(&st, buf.unwrap_or(0) as i32)
            });
            Ok(())
        });

    let mut dict = Dictionary::from_iter([
        ("version", Object::from(env!("CARGO_PKG_VERSION"))),
        ("setup", Object::from(setup)),
        ("update_isk", Object::from(update_isk)),
        ("nmap", Object::from(nmap)),
        ("xmap", Object::from(xmap)),
        ("omap", Object::from(omap)),
        ("imap", Object::from(imap)),
        ("preview_nmap", Object::from(preview_nmap)),
        ("default_keymaps", Object::from(default_keymaps)),
    ]);

    // One `jieba_preview_{motion}()` per previewable motion, plus the cancel.
    //
    // Exported as functions so a config can bind the preview directly instead of
    // routing through a `<Plug>(Jieba_preview_*)` string rhs - which silently
    // needs `noremap = false` to resolve, since a noremap mapping never expands
    // a `<Plug>` in its rhs. The `<Plug>` maps still exist for configs that
    // prefer them.
    for (name, motion) in [
        ("w", b"w".as_slice()),
        ("W", b"W".as_slice()),
        ("e", b"e".as_slice()),
        ("E", b"E".as_slice()),
        ("b", b"b".as_slice()),
        ("B", b"B".as_slice()),
        ("ge", b"ge".as_slice()),
        ("gE", b"gE".as_slice()),
    ] {
        let s = Rc::clone(&state);
        let f: Function<(), ()> = Function::from_fn(move |()| -> Result<()> {
            guard("preview", || preview::show(&s, motion));
            Ok(())
        });
        dict.insert(format!("jieba_preview_{name}"), Object::from(f));
    }
    let s = Rc::clone(&state);
    let cancel: Function<(), ()> = Function::from_fn(move |()| -> Result<()> {
        guard("preview_cancel", || preview::cancel(&s));
        Ok(())
    });
    dict.insert("jieba_preview_cancel", Object::from(cancel));

    Ok(dict)
}
