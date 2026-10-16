UNAME_S := $(shell uname -s)

ifeq ($(OS),Windows_NT)
  LIB  := target/release/jieba_rs_nvim.dll
  SO   := lua/jieba_rs.dll
else ifeq ($(UNAME_S),Darwin)
  LIB  := target/release/libjieba_rs_nvim.dylib
  SO   := lua/jieba_rs.so
else
  LIB  := target/release/libjieba_rs_nvim.so
  SO   := lua/jieba_rs.so
endif

CARGO_COLOR := $(if $(VIMRUNTIME),never,auto)

.PHONY: all build install test integration fmt clippy lint clean

all: install

## Compile the cdylib.
build:
	cargo build --release --color=$(CARGO_COLOR)

## Neovim's Lua C loader wants `lua/jieba_rs.so`, not `lib<crate>.so`.
## Install via a temp file + rename so a .so this nvim already dlopen'd is never
## clobbered in place (rename swaps the dirent and leaves the running inode
## mapped until restart).
install: build
	@mkdir -p lua
	cp $(LIB) $(SO).tmp
	mv $(SO).tmp $(SO)

## Engine unit tests (tokenizer, motions, golden master, imap arrows).
test:
	cargo test --release --color=$(CARGO_COLOR) --workspace

## Differential harness: real headless nvim, plugin maps vs native vim motions.
integration: install
	cargo test --release --color=$(CARGO_COLOR) --test integration

fmt:
	cargo fmt --all

clippy:
	cargo clippy --color=$(CARGO_COLOR) --workspace --all-targets

lint: fmt clippy

clean:
	cargo clean
	rm -f lua/jieba_rs.so lua/jieba_rs.dll lua/jieba_rs.dylib
