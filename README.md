# tokio-ar

[![Crates.io](https://img.shields.io/crates/v/tokio-ar.svg)](https://crates.io/crates/tokio-ar)
[![Documentation](https://docs.rs/tokio-ar/badge.svg)](https://docs.rs/tokio-ar)

A rust library for encoding/decoding Unix archive (.a) files.

Documentation: https://docs.rs/tokio-ar

## Overview

The `tokio-ar` crate is a pure Rust implementation of a
[Unix archive file](https://en.wikipedia.org/wiki/Ar_(Unix)) reader and writer.
This library provides a streaming interface, similar to that of the
[`tar`](https://crates.io/crates/tar) crate, that avoids having to ever load a
full archive entry into memory.

It's a fork of Matthew D. Steele's [`rust-ar`](https://github.com/mdsteele/rust-ar). 🖤

## License

tokio-ar is made available under the
[MIT License](http://spdx.org/licenses/MIT.html).
