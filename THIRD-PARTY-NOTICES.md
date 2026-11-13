# Third-party notices

ClipWheel uses the Rust standard library and the `windows` Rust bindings and their dependencies. See `Cargo.lock` for exact versions. These projects are distributed under their respective MIT and/or Apache-2.0 licenses (with applicable LLVM exceptions for runtime components).

The portable LLVM build includes `libunwind.dll` from the official LLVM-MinGW 20261020 distribution. Its accompanying license notices are included in `LLVM-LICENSE.txt` in the release folder. Do not remove that file when redistributing this build.

Source projects:
- https://github.com/microsoft/windows-rs
- https://github.com/rust-lang/rust
- https://github.com/mstorsjo/llvm-mingw
- https://llvm.org/
