//! The filesystem helpers the tool shares: the size of a file, and writing one atomically.

use std::fs;
use std::io;
use std::path::Path;

/// The size of a file in bytes, or zero when it cannot be read.
///
/// Every caller uses it to tell an empty file from one with frames or bytes in it, never to fail:
/// an unreadable recording is a dictation that captured nothing, not an error of its own.
pub fn file_size(path: &Path) -> u64 {
    fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0)
}

/// Replace a file's contents atomically: a reader never sees a half-written file, which a
/// truncating write would show it.
///
/// The temporary file carries this process's pid, so two writers cannot collide, and the rename
/// is what makes the new contents appear whole.
pub fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    let temporary = path.with_extension(format!("tmp.{}", std::process::id()));

    fs::write(&temporary, contents)?;
    fs::rename(&temporary, path)
}
