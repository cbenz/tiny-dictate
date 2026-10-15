# AUR package

`tiny-dictate-bin` installs the archive the Release workflow attaches to a GitHub release. Nothing
is compiled: the package depends on `gtk4` and `gtk4-layer-shell`, so `pacman` brings the shared
libraries the binary links and the pill draws with.

This directory holds the packaging source; the AUR itself is a separate git repository.

## Publish a new version

1. The release must exist: tag `v<version>` and let the Release workflow attach
   `tiny-dictate-<version>-x86_64-linux.tar.gz`.
2. Bump `pkgver` in `PKGBUILD`, and reset `pkgrel` to 1.
3. `updpkgsums` — it downloads the archive and writes the real `sha256sums` (from `SKIP`).
4. `makepkg --printsrcinfo > .SRCINFO`.
5. Push to the AUR:

   ```bash
   git clone ssh://aur@aur.archlinux.org/tiny-dictate-bin.git
   cp PKGBUILD .SRCINFO tiny-dictate-bin/
   cd tiny-dictate-bin && git commit -am "upgpkg: <version>-1" && git push
   ```

Users then install with `yay -S tiny-dictate-bin` (or any AUR helper), without a compiler.
