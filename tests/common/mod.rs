use std::path::{Path, PathBuf};

pub fn reader_fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/readers")
        .join(relative)
}
