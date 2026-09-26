use std::path::PathBuf;

fn home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

pub fn data_dir() -> PathBuf {
    home().join(".local/share/spydog")
}

pub fn config_path() -> PathBuf {
    home().join(".config/spydog/config.toml")
}

// Étend les variables d'environnement ($VAR) dans une chaîne.
pub fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        let mut name = String::new();
        while let Some(&n) = it.peek() {
            if n.is_ascii_alphanumeric() || n == '_' {
                name.push(n);
                it.next();
            } else {
                break;
            }
        }
        if name.is_empty() {
            out.push('$');
        } else {
            out.push_str(&std::env::var(&name).unwrap_or_default());
        }
    }
    out
}

pub fn epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_env_vars() {
        assert_eq!(expand_env("/a/b"), "/a/b");
        assert_eq!(
            expand_env("$HOME/x"),
            format!("{}/x", std::env::var("HOME").unwrap())
        );
    }
}
