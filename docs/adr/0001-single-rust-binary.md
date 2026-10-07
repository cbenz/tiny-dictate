# One Rust binary for the core, the audio pipeline and the pill

The core, the audio pipeline and the pill become a single Rust binary (edition 2024, `Cargo.lock`
committed, `rust-toolchain.toml`, clippy with `-D warnings`). The pill is a mode of that binary,
`_present`, spawned by the session as a child process, linking GTK4 and gtk4-layer-shell directly.

Measured during design in a spike: the layer-shell surface works with plain linkage, with no
`LD_PRELOAD` and no `build.rs`, because cargo already lists `libgtk4-layer-shell.so.0` before
`libwayland-client`; the pill reaches its first frame in 182 ms median, 118 ms with
`GSK_RENDERER=cairo`, against 361 ms for the Python presenter.

The design note also claimed that `exec` to `main` costs 3.6 ms even with GTK linked, and that
number does not hold on the machine the port was built on. Measured there (Intel i5-7300U,
2026-10-07): `tiny-dictate status` takes 31 ms median over 60 runs (29 ms minimum, 41 ms p90),
where `/bin/true` takes 0.6 ms. The difference is the dynamic loader relocating the GUI toolkit, and
`-z lazy` does not bring it down. The interactive commands therefore pay a loader cost the design
did not foresee; the port chose that over installing a presenter beside the binary, and it is the
reason the pill's own startup time is not the whole story.

## Consequences

The GUI toolkit is linked into every invocation, `tiny-dictate status` included, and costs the loader
time measured above. That is deliberate; it is not a packaging mistake to fix.
