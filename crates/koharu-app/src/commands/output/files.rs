use std::{
    io::{self, Write as _},
    path::Path,
};

pub(super) fn is_complete(path: &Path) -> io::Result<bool> {
    match path.metadata() {
        Ok(metadata) if metadata.is_file() => Ok(metadata.len() > 0),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "export path is not a file",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

pub(super) fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "export path has no parent"))?;
    // Publish only complete pages: interrupted writes must not become resume markers.
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_skips_completed_files_but_retries_empty_or_missing_files() {
        let directory = tempfile::tempdir().unwrap();
        let complete = directory.path().join("0001_page.png");
        let empty = directory.path().join("0002_page.png");
        let missing = directory.path().join("0003_page.png");
        std::fs::write(&complete, b"completed page").unwrap();
        std::fs::write(&empty, []).unwrap();
        for path in [&complete, &empty, &missing] {
            if !is_complete(path).unwrap() {
                write(path, b"new page").unwrap();
            }
        }
        assert_eq!(std::fs::read(complete).unwrap(), b"completed page");
        assert_eq!(std::fs::read(empty).unwrap(), b"new page");
        assert_eq!(std::fs::read(missing).unwrap(), b"new page");
    }

    #[test]
    fn a_directory_is_not_a_completed_export() {
        let directory = tempfile::tempdir().unwrap();
        assert!(is_complete(directory.path()).is_err());
    }
}
