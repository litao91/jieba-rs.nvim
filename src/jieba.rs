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

//! The jieba segmenter behind the tokenizer.

use std::fs::File;
use std::io::BufReader;
use std::sync::OnceLock;

use jieba_rs::Jieba;
use jieba_vim_rs_core::token::JiebaPlaceholder;

/// One concrete segmenter type for both the lazy and eager configurations.
///
/// `OnceLock` makes eager loading a degenerate case (initialize in `new`), so
/// the plugin never needs a `Box<dyn JiebaPlaceholder>` or an enum here - the
/// tokenizer hot path stays monomorphic.
///
/// A user dictionary *replaces* the built-in one (`Jieba::with_dict`), matching
/// upstream jieba.vim's `g:jieba_vim_user_dict` semantics.
pub struct JiebaImpl {
    jieba: OnceLock<Jieba>,
}

impl JiebaImpl {
    /// Build the segmenter. With `eager` false and no `path`, the ~5 MB built-in
    /// dictionary is only parsed on the first line that actually contains
    /// Chinese. Anything that can fail (opening and parsing a user dictionary)
    /// happens here so `setup()` reports it once, up front, instead of panicking
    /// from inside a keymap callback later.
    pub fn new(path: Option<String>, eager: bool) -> Result<Self, String> {
        let jieba = OnceLock::new();
        match path {
            Some(path) => {
                let mut reader =
                    BufReader::new(File::open(&path).map_err(|err| {
                        format!("failed to open user dict `{path}`: {err}")
                    })?);
                let dict = Jieba::with_dict(&mut reader).map_err(|err| {
                    format!("failed to parse user dict `{path}`: {err}")
                })?;
                let _ = jieba.set(dict);
            }
            None if eager => {
                let _ = jieba.set(Jieba::new());
            }
            None => {}
        }
        Ok(Self { jieba })
    }
}

impl JiebaPlaceholder for JiebaImpl {
    fn cut_hmm_into_char_counts(&self, sentence: &str) -> Vec<usize> {
        self.jieba
            .get_or_init(Jieba::new)
            .cut(sentence, true)
            .into_iter()
            .map(|token| token.end - token.start)
            .collect()
    }
}
