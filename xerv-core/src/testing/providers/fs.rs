//! Filesystem provider for abstracting file operations.
//!
//! Allows tests to use an in-memory filesystem while production code
//! uses the real filesystem.

use parking_lot::RwLock;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

/// Provider trait for filesystem operations.
pub trait FsProvider: Send + Sync {
    /// Read file contents.
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;

    /// Write file contents.
    fn write(&self, path: &Path, contents: &[u8]) -> io::Result<()>;

    /// Check if a path exists.
    fn exists(&self, path: &Path) -> bool;

    /// Check if a path is a file.
    fn is_file(&self, path: &Path) -> bool;

    /// Check if a path is a directory.
    fn is_dir(&self, path: &Path) -> bool;

    /// Create a directory and all parent directories.
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;

    /// Remove a file.
    fn remove_file(&self, path: &Path) -> io::Result<()>;

    /// Remove a directory and all its contents.
    fn remove_dir_all(&self, path: &Path) -> io::Result<()>;

    /// List directory contents.
    fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>>;

    /// Get file metadata.
    fn metadata(&self, path: &Path) -> io::Result<FsMetadata>;

    /// Check if this is a mock provider.
    fn is_mock(&self) -> bool;
}

/// File metadata.
#[derive(Debug, Clone)]
pub struct FsMetadata {
    /// Size of the file or directory in bytes.
    pub size: u64,
    /// Whether the path refers to a regular file.
    pub is_file: bool,
    /// Whether the path refers to a directory.
    pub is_dir: bool,
}

/// Real filesystem provider.
pub struct RealFs;

impl RealFs {
    /// Create a new real filesystem provider.
    pub fn new() -> Self {
        Self
    }
}

impl Default for RealFs {
    fn default() -> Self {
        Self::new()
    }
}

impl FsProvider for RealFs {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn write(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        std::fs::write(path, contents)
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }

    fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_dir_all(path)
    }

    fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        std::fs::read_dir(path)?
            .map(|entry| entry.map(|e| e.path()))
            .collect()
    }

    fn metadata(&self, path: &Path) -> io::Result<FsMetadata> {
        let meta = std::fs::metadata(path)?;
        Ok(FsMetadata {
            size: meta.len(),
            is_file: meta.is_file(),
            is_dir: meta.is_dir(),
        })
    }

    fn is_mock(&self) -> bool {
        false
    }
}

/// In-memory filesystem for testing.
///
/// # Example
///
/// ```
/// use xerv_core::testing::{MockFs, FsProvider};
/// use std::path::Path;
///
/// let fs = MockFs::new()
///     .with_file("/config/app.yaml", b"name: test")
///     .with_file("/data/input.json", b"{}");
///
/// assert!(fs.exists(Path::new("/config/app.yaml")));
/// assert_eq!(fs.read(Path::new("/config/app.yaml")).unwrap(), b"name: test");
/// ```
pub struct MockFs {
    files: RwLock<HashMap<PathBuf, Vec<u8>>>,
    dirs: RwLock<HashMap<PathBuf, ()>>,
}

impl MockFs {
    /// Create a new empty mock filesystem.
    pub fn new() -> Self {
        let mut dirs = HashMap::new();
        // Root directory always exists
        dirs.insert(PathBuf::from("/"), ());
        Self {
            files: RwLock::new(HashMap::new()),
            dirs: RwLock::new(dirs),
        }
    }

    /// Add a file to the mock filesystem.
    pub fn with_file(self, path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> Self {
        let path = path.as_ref().to_path_buf();

        // Ensure parent directories exist
        if let Some(parent) = path.parent() {
            self.ensure_parent_dirs(parent);
        }

        self.files.write().insert(path, contents.as_ref().to_vec());
        self
    }

    /// Add a file with string contents.
    pub fn with_text_file(self, path: impl AsRef<Path>, contents: &str) -> Self {
        self.with_file(path, contents.as_bytes())
    }

    /// Add a directory to the mock filesystem.
    pub fn with_dir(self, path: impl AsRef<Path>) -> Self {
        self.ensure_parent_dirs(path.as_ref());
        self.dirs.write().insert(path.as_ref().to_path_buf(), ());
        self
    }

    /// Ensure all parent directories exist.
    fn ensure_parent_dirs(&self, path: &Path) {
        let mut dirs = self.dirs.write();
        let mut current = PathBuf::new();
        for component in path.components() {
            current.push(component);
            dirs.entry(current.clone()).or_insert(());
        }
    }

    /// Get all files in the mock filesystem.
    pub fn all_files(&self) -> Vec<PathBuf> {
        self.files.read().keys().cloned().collect()
    }

    /// Get all directories in the mock filesystem.
    pub fn all_dirs(&self) -> Vec<PathBuf> {
        self.dirs.read().keys().cloned().collect()
    }

    /// Clear the mock filesystem.
    pub fn clear(&self) {
        self.files.write().clear();
        let mut dirs = self.dirs.write();
        dirs.clear();
        dirs.insert(PathBuf::from("/"), ());
    }

    /// Normalize a path (remove . and ..).
    fn normalize_path(path: &Path) -> PathBuf {
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                std::path::Component::ParentDir => {
                    normalized.pop();
                }
                std::path::Component::CurDir => {}
                _ => normalized.push(component),
            }
        }
        normalized
    }
}

impl Default for MockFs {
    fn default() -> Self {
        Self::new()
    }
}

impl FsProvider for MockFs {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let path = Self::normalize_path(path);
        self.files.read().get(&path).cloned().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("File not found: {}", path.display()),
            )
        })
    }

    fn write(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        let path = Self::normalize_path(path);

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() && !self.dirs.read().contains_key(parent) {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("Parent directory not found: {}", parent.display()),
                ));
            }
        }

        self.files.write().insert(path, contents.to_vec());
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        let path = Self::normalize_path(path);
        self.files.read().contains_key(&path) || self.dirs.read().contains_key(&path)
    }

    fn is_file(&self, path: &Path) -> bool {
        let path = Self::normalize_path(path);
        self.files.read().contains_key(&path)
    }

    fn is_dir(&self, path: &Path) -> bool {
        let path = Self::normalize_path(path);
        self.dirs.read().contains_key(&path)
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        self.ensure_parent_dirs(&Self::normalize_path(path));
        Ok(())
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        let path = Self::normalize_path(path);
        self.files
            .write()
            .remove(&path)
            .map(|_| ())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "File not found"))
    }

    fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
        let path = Self::normalize_path(path);

        // Remove all files under this directory
        self.files.write().retain(|p, _| !p.starts_with(&path));

        // Remove all directories under this directory
        self.dirs.write().retain(|p, _| !p.starts_with(&path));

        Ok(())
    }

    fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        let path = Self::normalize_path(path);

        if !self.dirs.read().contains_key(&path) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Directory not found",
            ));
        }

        let mut entries = Vec::new();

        // Add files directly under this directory
        for file_path in self.files.read().keys() {
            if let Some(parent) = file_path.parent() {
                if parent == path {
                    entries.push(file_path.clone());
                }
            }
        }

        // Add directories directly under this directory
        for dir_path in self.dirs.read().keys() {
            if let Some(parent) = dir_path.parent() {
                if parent == path && dir_path != &path {
                    entries.push(dir_path.clone());
                }
            }
        }

        Ok(entries)
    }

    fn metadata(&self, path: &Path) -> io::Result<FsMetadata> {
        let path = Self::normalize_path(path);

        if let Some(contents) = self.files.read().get(&path) {
            return Ok(FsMetadata {
                size: contents.len() as u64,
                is_file: true,
                is_dir: false,
            });
        }

        if self.dirs.read().contains_key(&path) {
            return Ok(FsMetadata {
                size: 0,
                is_file: false,
                is_dir: true,
            });
        }

        Err(io::Error::new(io::ErrorKind::NotFound, "Path not found"))
    }

    fn is_mock(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_fs_basic_operations() {
        let fs = MockFs::new()
            .with_file("/test.txt", b"hello world")
            .with_dir("/data");

        assert!(fs.exists(Path::new("/test.txt")));
        assert!(fs.is_file(Path::new("/test.txt")));
        assert!(!fs.is_dir(Path::new("/test.txt")));

        assert!(fs.exists(Path::new("/data")));
        assert!(fs.is_dir(Path::new("/data")));
        assert!(!fs.is_file(Path::new("/data")));

        let contents = fs.read(Path::new("/test.txt")).unwrap();
        assert_eq!(contents, b"hello world");
    }

    #[test]
    fn mock_fs_write() {
        let fs = MockFs::new().with_dir("/data");

        fs.write(Path::new("/data/file.txt"), b"test content")
            .unwrap();

        assert!(fs.exists(Path::new("/data/file.txt")));
        assert_eq!(
            fs.read(Path::new("/data/file.txt")).unwrap(),
            b"test content"
        );
    }

    #[test]
    fn mock_fs_remove() {
        let fs = MockFs::new()
            .with_file("/file.txt", b"test")
            .with_file("/dir/nested.txt", b"nested")
            .with_dir("/dir");

        fs.remove_file(Path::new("/file.txt")).unwrap();
        assert!(!fs.exists(Path::new("/file.txt")));

        fs.remove_dir_all(Path::new("/dir")).unwrap();
        assert!(!fs.exists(Path::new("/dir")));
        assert!(!fs.exists(Path::new("/dir/nested.txt")));
    }

    #[test]
    fn mock_fs_read_dir() {
        let fs = MockFs::new()
            .with_file("/dir/a.txt", b"a")
            .with_file("/dir/b.txt", b"b")
            .with_dir("/dir/subdir");

        let entries = fs.read_dir(Path::new("/dir")).unwrap();
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn mock_fs_metadata() {
        let fs = MockFs::new()
            .with_file("/file.txt", b"12345")
            .with_dir("/dir");

        let file_meta = fs.metadata(Path::new("/file.txt")).unwrap();
        assert_eq!(file_meta.size, 5);
        assert!(file_meta.is_file);
        assert!(!file_meta.is_dir);

        let dir_meta = fs.metadata(Path::new("/dir")).unwrap();
        assert!(dir_meta.is_dir);
        assert!(!dir_meta.is_file);
    }

    #[test]
    fn mock_fs_auto_creates_parent_dirs() {
        let fs = MockFs::new().with_file("/a/b/c/file.txt", b"deep");

        assert!(fs.is_dir(Path::new("/a")));
        assert!(fs.is_dir(Path::new("/a/b")));
        assert!(fs.is_dir(Path::new("/a/b/c")));
        assert!(fs.is_file(Path::new("/a/b/c/file.txt")));
    }
}
