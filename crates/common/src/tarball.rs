//! In-memory unpacking of GitHub branch tarballs (snapshot transport between
//! bot and reconciler). GitHub wraps everything in a `<org>-<repo>-<sha7>/`
//! top-level directory — we strip it.

use anyhow::Result;
use flate2::read::GzDecoder;
use std::collections::BTreeMap;
use std::io::Read;

pub fn untar(gzipped: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut archive = tar::Archive::new(GzDecoder::new(gzipped));
    let mut files = BTreeMap::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path()?.into_owned();
        let stripped: std::path::PathBuf = path.components().skip(1).collect();
        if stripped.as_os_str().is_empty() {
            continue;
        }
        let mut content = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut content)?;
        files.insert(stripped.to_string_lossy().into_owned(), content);
    }
    Ok(files)
}
