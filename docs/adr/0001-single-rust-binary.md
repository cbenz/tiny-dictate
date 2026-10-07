# One Rust binary for the core, the audio pipeline and the pill

The core, the audio pipeline and the pill become a single Rust binary (edition 2024, `Cargo.lock`
committed, `rust-toolchain.toml`, clippy with `-D warnings`). The pill is a mode of that binary,
`_present`, spawned by the session as a child process, linking GTK4 and gtk4-layer-shell directly.

Measured during design in a spike: the layer-shell surface works with plain linkage, with no
`LD_PRELOAD` and no `build.rs`, because cargo already lists `libgtk4-layer-shell.so.0` before
`libwayland-client`; the pill reaches its first frame in 182 ms median, 118 ms with
`GSK_RENDERER=cairo`, against 361 ms for the Python presenter; and `exec` to `main` costs 3.6 ms even
with GTK linked. One binary therefore does not slow the interactive commands down, and nothing has to
be installed beside it.

## Consequences

The GUI toolkit is linked into every invocation, `tiny-dictate status` included. That is deliberate;
it is not a packaging mistake to fix.
