use std::path::{Component, Path, PathBuf};

pub fn normalize_relative_path(raw: &str) -> Result<PathBuf, String> {
    let normalized = raw.replace('\\', "/");
    let raw_path = Path::new(&normalized);

    if raw_path.is_absolute() {
        return Err("absolute paths are not allowed".to_string());
    }

    let mut clean = PathBuf::new();
    for component in raw_path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => clean.push(part),
            Component::ParentDir => {
                return Err("path traversal is not allowed".to_string());
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err("absolute paths are not allowed".to_string());
            }
        }
    }

    Ok(clean)
}

#[cfg(test)]
mod tests {
    use super::normalize_relative_path;
    use std::path::PathBuf;

    #[test]
    fn normalizes_relative_paths() {
        assert_eq!(
            normalize_relative_path(r"folder\child/file.txt").unwrap(),
            PathBuf::from("folder").join("child").join("file.txt")
        );
        assert_eq!(normalize_relative_path(".").unwrap(), PathBuf::new());
    }

    #[test]
    fn rejects_path_traversal() {
        assert!(normalize_relative_path("../escape").is_err());
        assert!(normalize_relative_path(r"\absolute").is_err());
    }
}
