use anyhow::Result;
use std::path::PathBuf;
use walkdir::WalkDir;

pub fn find_tf_files(root: &PathBuf) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in WalkDir::new(root).follow_links(true) {
        let entry = entry?;
        if entry.file_type().is_file() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("tf") {
                files.push(path.to_path_buf());
            }
        }
    }
    Ok(files)
}
