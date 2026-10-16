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

//! Differential integration harness.
//!
//! The plugin's contract is that on **pure ASCII** text its word motions and
//! text objects are indistinguishable from vim's builtins (Chinese is where it
//! is supposed to differ). Rather than hand-maintain golden expectations, this
//! runs every case twice - once with the plugin's mappings installed, once in a
//! vanilla nvim - and compares the resulting editor state.
//!
//! Each run is a **fresh `nvim --headless` process**, so nothing can leak
//! between cases (no stale typeahead, no leftover marks or visual state). That
//! is the property a single-process harness cannot give, and getting it wrong
//! produces failures that look like plugin bugs but are harness contamination.
//!
//! Run with `make integration` (which builds and installs `lua/jieba_rs.so`
//! first), or directly:
//!
//! ```sh
//! cargo test --release --test integration
//! ```
//!
//! Skips itself when `nvim` is not on `PATH`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// One editor scenario: a starting buffer, a cursor position, and keystrokes.
struct Case {
    lines: &'static [&'static str],
    lnum: usize,
    /// 1-based byte column.
    col: usize,
    /// Keystrokes, with `<...>` key notation left unescaped (the driver escapes
    /// it into vimscript's `\<...>` form).
    keys: &'static str,
}

impl Case {
    fn label(&self) -> String {
        format!("[{}:{}] {:?}", self.lnum, self.col, self.keys)
    }
}

/// ASCII-only buffers: this is where the plugin must match vim exactly.
fn cases() -> Vec<Case> {
    let mut v = Vec::new();
    let mut add = |lines: &'static [&'static str],
                   lnum: usize,
                   col: usize,
                   keys: &'static str| {
        v.push(Case {
            lines,
            lnum,
            col,
            keys,
        })
    };

    let one: &'static [&'static str] = &["hello world foo bar"];
    let punct: &'static [&'static str] = &["hello, world; foo! bar"];
    let multi: &'static [&'static str] = &["foo bar baz", "qux quux", "corge"];
    let spaces: &'static [&'static str] = &["   hello world", "  "];
    let short: &'static [&'static str] = &["hi"];

    // --- plain motions ---
    for keys in ["w", "W", "e", "E", "b", "B", "ge", "gE"] {
        for (lines, lnum, col) in [
            (one, 1, 1),
            (one, 1, 3),
            (one, 1, 7),
            (one, 1, 19),
            (punct, 1, 1),
            (punct, 1, 6),
            (punct, 1, 8),
            (multi, 1, 5),
            (multi, 2, 1),
            (multi, 3, 5),
            (spaces, 1, 1),
            (spaces, 2, 3),
            (short, 1, 1),
            (short, 1, 2),
        ] {
            add(lines, lnum, col, keys);
        }
    }

    // --- counts ---
    for keys in ["2w", "3w", "2e", "2b", "2ge", "4W", "10w"] {
        add(one, 1, 1, keys);
        add(multi, 1, 1, keys);
        add(punct, 1, 1, keys);
    }

    // --- operators over motions ---
    for keys in [
        "dw", "dW", "de", "db", "dge", "d2w", "d3e", "yw", "yW", "ye", "y2w",
        "gUiw", "guiw",
    ] {
        add(one, 1, 1, keys);
        add(one, 1, 7, keys);
        add(punct, 1, 1, keys);
        add(multi, 1, 5, keys);
    }

    // --- text objects ---
    for keys in [
        "diw", "daw", "diW", "daW", "yiw", "yaw", "ciwX", "cawZ", "gUiw",
        "2diw",
    ] {
        add(one, 1, 1, keys);
        add(one, 1, 3, keys);
        add(one, 1, 7, keys);
        add(one, 1, 13, keys);
        add(punct, 1, 6, keys);
        add(multi, 2, 2, keys);
        add(spaces, 1, 5, keys);
    }

    // --- visual mode ---
    for keys in [
        "vw", "v2w", "ve", "vb", "viw", "vaw", "viW", "vaW", "vwh", "vge",
        "Vw", "Vj", "v\x1bw", "vawy", "viwy", "vw\x1b", "viw\x1b",
    ] {
        add(one, 1, 1, keys);
        add(one, 1, 7, keys);
        add(multi, 1, 5, keys);
        add(punct, 1, 6, keys);
    }
    // Blockwise visual: the mode byte is \x16, which is not valid UTF-8, so the
    // whole keymap/normal-command path has to carry it as bytes.
    for keys in ["v\u{16}w", "v\u{16}iw"] {
        add(one, 1, 1, keys);
    }

    // --- arrow keys (normal / visual / insert) ---
    for keys in [
        "<C-Right>",
        "<C-Left>",
        "<S-Right>",
        "<S-Left>",
        "2<C-Right>",
        "v<C-Right>",
        "v<S-Left>",
    ] {
        add(one, 1, 1, keys);
        add(one, 1, 13, keys);
        add(multi, 2, 1, keys);
    }
    for keys in [
        "i<C-Right>",
        "i<C-Left>",
        "i<S-Right>",
        "i<S-Left>",
        "i<C-w>",
        "i<C-w><C-w>",
    ] {
        add(one, 1, 13, keys);
        add(one, 1, 1, keys);
        add(multi, 2, 4, keys);
    }

    // --- cross-line and edge cases ---
    add(multi, 1, 9, "w");
    add(multi, 1, 9, "dw");
    add(multi, 3, 5, "w");
    add(multi, 3, 5, "e");
    add(spaces, 2, 3, "w");
    add(&[""], 1, 1, "w");
    add(&[""], 1, 1, "dw");
    add(&["a"], 1, 1, "w");
    add(&["a"], 1, 1, "dw");
    add(&["a"], 1, 1, "diw");

    v
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// Escape a Rust string as a vimscript **single-quoted** literal (only `'` needs
/// doubling; everything else, including backslashes, is literal).
fn vim_single_quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Build the `-c` arguments that set up the buffer, optionally install the
/// plugin, feed the keys, and dump the resulting state to `out`.
fn driver_args(case: &Case, plugin: bool, out: &Path) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();

    // `enew` leaves one empty line, and `setline(1, [...])` overwrites it, so
    // the buffer ends up exactly as long as the case's line list.
    let lines = case
        .lines
        .iter()
        .map(|l| vim_single_quoted(l))
        .collect::<Vec<_>>()
        .join(", ");
    args.push(format!("call setline(1, [{lines}])"));
    args.push(format!("call cursor({}, {})", case.lnum, case.col));

    if plugin {
        // `mappings = true` installs the default keymaps globally, so the same
        // literal keystrokes drive the plugin here and vim's builtins below.
        args.push(
            "lua require('jieba_rs').setup{ mappings = true }".to_string(),
        );
    }

    // A double-quoted vimscript literal so `<...>` key notation becomes real
    // termcodes; the backslash is the one we just inserted.
    let keys = case.keys.replace('<', "\\<");
    args.push(format!("call feedkeys(\"{keys}\", 'x')"));
    // A `c` operation ends in insert mode; leave it so the state dump is
    // comparable with the native run.
    args.push(
        "if mode(1)[0] ==# 'i' | call feedkeys(\"\\<Esc>\", 'x') | endif"
            .to_string(),
    );

    // Dump the state as `key=value` lines so Rust can compare selected fields.
    //
    // The selection is normalized: while still in Visual mode `'<`/`'>` are
    // unset (vim only writes them when visual mode *ends*) and the live
    // selection is `getpos('v')` .. cursor. The plugin has to leave and re-enter
    // visual mode to apply a computed selection, so it would otherwise report
    // `'<`/`'>` where the native run reports nothing - an artifact of the
    // implementation strategy, not a behavioural difference.
    //
    // `mode` is reduced to its first character's code point because blockwise
    // visual is the raw byte \x16, which is not valid UTF-8 and would break
    // reading the dump back as text.
    let visual = "index(['v', 'V', \"\\<C-v>\"], mode(1)[0]) >= 0";
    let out = vim_single_quoted(&out.to_string_lossy());
    args.push(format!(
        "call writefile([\
         'buf=' . join(getline(1, '$'), \"\\x01\"), \
         'cur=' . string(getcurpos()[1:2]), \
         'sel0=' . string({visual} ? getpos('v')[1:2] : getpos(\"'<\")[1:2]), \
         'sel1=' . string({visual} ? getcurpos()[1:2] : getpos(\"'>\")[1:2]), \
         'mode=' . char2nr(mode(1)[0]), \
         'reg=' . getreg('\"'), \
         'lb=' . string(getpos(\"'[\")[1:2]), \
         'rb=' . string(getpos(\"']\")[1:2])\
         ], {out})"
    ));
    args
}

/// The dumped state of one run, as `key -> value`.
type Dump = std::collections::BTreeMap<String, String>;

fn parse_dump(text: &str) -> Dump {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Whether `'[` / `']` are part of the contract for these keystrokes.
///
/// Operators set them through the real `d`/`c`/`y` the playback runs, so they
/// must match. Insert-mode deletions (`i_CTRL-W`) and plain visual selections do
/// not go through an operator, and there the plugin legitimately differs - as
/// upstream jieba.vim does - so comparing them would only add noise.
fn compares_marks(keys: &str) -> bool {
    if keys.starts_with(['i', 'v', 'V']) {
        return false;
    }
    ["d", "c", "y", "gU", "gu", "~"]
        .iter()
        .any(|op| keys.starts_with(op))
}

/// Run one case in a fresh headless nvim and return the dumped state.
fn run_case(case: &Case, plugin: bool, out: &Path) -> Option<Dump> {
    let root = repo_root();
    let mut cmd = Command::new("nvim");
    cmd.arg("--headless")
        .arg("-u")
        .arg("NONE")
        .arg("--cmd")
        .arg(format!("set rtp+={}", root.display()))
        .arg("--cmd")
        .arg("set noswapfile nomodeline noundofile")
        .arg("-c")
        .arg("enew");
    for a in driver_args(case, plugin, out) {
        cmd.arg("-c").arg(a);
    }
    cmd.arg("-c").arg("qa!");

    let output = cmd.output().ok()?;
    let dumped = std::fs::read_to_string(out).ok()?;
    let _ = std::fs::remove_file(out);
    if !output.status.success() && dumped.is_empty() {
        eprintln!(
            "nvim exited {} for {}: {}",
            output.status,
            case.label(),
            String::from_utf8_lossy(&output.stderr)
        );
        return None;
    }
    Some(parse_dump(&dumped))
}

/// The fields compared for every case, plus `'[`/`']` where they are part of
/// the contract.
fn compare_fields(keys: &str) -> Vec<&'static str> {
    let mut f = vec!["buf", "cur", "sel0", "sel1", "mode", "reg"];
    if compares_marks(keys) {
        f.push("lb");
        f.push("rb");
    }
    f
}

fn diff_fields(plugin: &Dump, native: &Dump, keys: &str) -> Vec<String> {
    compare_fields(keys)
        .into_iter()
        .filter(|f| plugin.get(*f) != native.get(*f))
        .map(|f| {
            format!(
                "    {f}: plugin={:?} native={:?}",
                plugin.get(f),
                native.get(f)
            )
        })
        .collect()
}

#[test]
fn ascii_matches_native_vim() {
    if Command::new("nvim").arg("--version").output().is_err() {
        eprintln!("skipping: nvim not found on PATH");
        return;
    }
    let so = repo_root().join("lua").join("jieba_rs.so");
    assert!(
        so.exists(),
        "lua/jieba_rs.so is missing; run `make install` first"
    );

    let tmp = std::env::temp_dir();
    let mut diffs: Vec<String> = Vec::new();
    let cases = cases();
    for (i, case) in cases.iter().enumerate() {
        let plug_out = tmp.join(format!("jieba_it_plug_{i}.out"));
        let nat_out = tmp.join(format!("jieba_it_nat_{i}.out"));
        let plugin = run_case(case, true, &plug_out);
        let native = run_case(case, false, &nat_out);
        match (plugin, native) {
            (Some(p), Some(n)) => {
                let d = diff_fields(&p, &n, case.keys);
                if !d.is_empty() {
                    diffs.push(format!(
                        "{}\n  buffer: {:?}\n{}",
                        case.label(),
                        case.lines,
                        d.join("\n")
                    ));
                }
            }
            _ => diffs.push(format!("{}: nvim run failed", case.label())),
        }
    }

    assert!(
        diffs.is_empty(),
        "{} of {} cases diverge from native vim on ASCII:\n\n{}",
        diffs.len(),
        cases.len(),
        diffs.join("\n\n")
    );
    eprintln!("all {} cases match native vim", cases.len());
}
