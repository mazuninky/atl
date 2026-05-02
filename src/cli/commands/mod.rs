pub mod alias;
pub mod api;
pub mod auth;
pub mod browse;
pub mod config;
pub mod confluence;
pub mod confluence_url;
pub mod docs;
pub mod init;
pub mod jira;
pub mod markdown;
pub mod updater;
pub mod updater_notifier;

use std::io::{self, Read};

pub fn read_body_arg(arg: &str) -> anyhow::Result<String> {
    if arg == "-" {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        Ok(buf)
    } else if let Some(path) = arg.strip_prefix('@') {
        if path.is_empty() {
            anyhow::bail!("body file path after '@' cannot be empty");
        }
        Ok(std::fs::read_to_string(path)?)
    } else {
        Ok(arg.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_body_arg_literal() {
        let result = read_body_arg("hello world").unwrap();
        assert_eq!(result, "hello world");
    }

    #[test]
    fn read_body_arg_from_file() {
        let dir = std::env::temp_dir();
        let path = dir.join("atl_test_body.txt");
        std::fs::write(&path, "file content").unwrap();

        let arg = format!("@{}", path.display());
        let result = read_body_arg(&arg).unwrap();
        assert_eq!(result, "file content");

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn read_body_arg_from_missing_file() {
        let result = read_body_arg("@/nonexistent/path/file.txt");
        assert!(result.is_err());
    }
}
