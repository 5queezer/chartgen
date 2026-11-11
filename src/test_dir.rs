//! Directories for tests. Each is under the build's target directory, so on a disk and
//! not in the system's temporary directory, which is memory on many machines; and each
//! is removed when the test that made it ends, also when that test fails.
//!
//! Test code only. The unit tests of the library and of the binary and the integration
//! tests are three crates, so each includes this one file.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

/// What a test thread made, removed when the thread ends: the test harness runs every
/// test on a thread of its own, and a thread's locals are dropped when it ends, after a
/// panic too.
struct Made(Vec<PathBuf>);

impl Drop for Made {
    fn drop(&mut self) {
        for dir in &self.0 {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

thread_local! {
    static MADE: RefCell<Made> = const { RefCell::new(Made(Vec::new())) };
}

/// A new, empty directory for the calling test, named after `name`. **Call it on the
/// test's own thread:** the directory goes when the calling thread ends.
pub fn new(name: impl AsRef<str>) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = root().join(format!(
        "{}-{}-{}",
        name.as_ref(),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    MADE.with(|made| made.borrow_mut().0.push(dir.clone()));
    dir
}

/// `<target>/tmp/chartgen-tests`, found from where the test binary is
/// (`<target>/<profile>/deps/`). Cargo offers `CARGO_TARGET_TMPDIR` to integration
/// tests only, and this serves unit tests too. Anywhere else, the system's directory.
fn root() -> PathBuf {
    let exe = std::env::current_exe().ok();
    let target = exe
        .as_deref()
        .and_then(|exe| {
            exe.ancestors()
                .find(|a| a.file_name().is_some_and(|n| n == "deps"))
        })
        .and_then(|deps| deps.parent())
        .and_then(|profile| profile.parent());
    match target {
        Some(target) => target.join("tmp").join("chartgen-tests"),
        None => std::env::temp_dir().join("chartgen-tests"),
    }
}
