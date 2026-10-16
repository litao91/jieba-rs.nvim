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

//! The buffer the engine reads through.

use std::cell::RefCell;
use std::collections::HashMap;

use jieba_vim_rs_core::BufferLike;
use nvim_oxi::api::Buffer;

/// A [`BufferLike`] view of a Neovim buffer, valid for one motion.
///
/// `BufferLike::getline` is called once per line the engine walks, so without a
/// cache a long `W` across a large buffer would cost one
/// `nvim_buf_get_lines` round trip per step. Nothing edits the buffer while a
/// model call runs, so the snapshot cannot go stale: build a fresh one per
/// mapping invocation and drop it afterwards.
pub struct NvimBuffer {
    buf: Buffer,
    lines: RefCell<Option<usize>>,
    cache: RefCell<HashMap<usize, String>>,
}

impl NvimBuffer {
    pub fn new(buf: Buffer) -> Self {
        Self {
            buf,
            lines: RefCell::new(None),
            cache: RefCell::new(HashMap::new()),
        }
    }

    pub fn handle(&self) -> i32 {
        self.buf.handle()
    }
}

impl BufferLike for NvimBuffer {
    type Error = String;

    fn getline(&self, lnum: usize) -> Result<String, Self::Error> {
        if let Some(line) = self.cache.borrow().get(&lnum) {
            return Ok(line.clone());
        }
        let total = self.lines()?;
        // Upstream's Lua shim special-cased this: an empty buffer still has one
        // (empty) line as far as the engine is concerned.
        if total == 0 && lnum == 1 {
            return Ok(String::new());
        }
        if lnum == 0 || lnum > total {
            return Err(format!("line {lnum} out of range (1..={total})"));
        }
        let got = self
            .buf
            .get_lines(lnum - 1..lnum, false)
            .map_err(|err| format!("nvim_buf_get_lines failed: {err}"))?;
        // Buffer text is UTF-8, but nvim's String is a byte buffer, so fall back
        // lossily rather than failing the whole motion on one odd line.
        let line = got
            .into_iter()
            .next()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.cache.borrow_mut().insert(lnum, line.clone());
        Ok(line)
    }

    fn lines(&self) -> Result<usize, Self::Error> {
        if let Some(n) = *self.lines.borrow() {
            return Ok(n);
        }
        let n = self
            .buf
            .line_count()
            .map_err(|err| format!("nvim_buf_line_count failed: {err}"))?;
        *self.lines.borrow_mut() = Some(n);
        Ok(n)
    }
}
