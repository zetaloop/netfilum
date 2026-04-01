pub fn windows_path_to_wsl(raw: &str) -> Result<String, String> {
    let bytes = raw.as_bytes();
    if bytes.len() < 3 || bytes[1] != b':' {
        return Err(format!("unsupported Windows path: {raw}"));
    }

    let drive = bytes[0] as char;
    let suffix = raw[2..].replace('\\', "/");
    Ok(format!(
        "/mnt/{}/{}",
        drive.to_ascii_lowercase(),
        suffix.trim_start_matches('/')
    ))
}

#[cfg(test)]
mod tests {
    use super::windows_path_to_wsl;

    #[test]
    fn converts_windows_path_to_wsl() {
        assert_eq!(
            windows_path_to_wsl(r"D:\GitHub\netfilum").unwrap(),
            "/mnt/d/GitHub/netfilum"
        );
    }
}
