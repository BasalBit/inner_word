//! Quoting for a POSIX shell (zsh, bash, sh): what a person pastes into Terminal.

/// `text` as one shell word.
///
/// A word of plain characters is returned as it is. Anything else is wrapped in single
/// quotes, inside which the shell changes nothing; a single quote in the text is written
/// as `'\''`. The path of the tool inside the app bundle contains a space ("Inner
/// Word.app"), so it always comes out quoted.
pub fn quote(text: &str) -> String {
    let plain = !text.is_empty()
        && text.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '/' | '.' | '_' | '-' | '+' | ':' | '@' | '%' | ',')
        });
    if plain {
        return text.to_string();
    }
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('\'');
    for c in text.chars() {
        if c == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(c);
        }
    }
    quoted.push('\'');
    quoted
}

/// `text` as a JSON string literal, quotes included. It is also a valid TOML basic string
/// and a valid YAML double-quoted scalar: the three share the escapes JSON produces.
pub fn string_literal(text: &str) -> String {
    serde_json::Value::String(text.to_string()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn plain_words_stay_as_they_are() {
        assert_eq!(quote("/usr/local/bin/innerword"), "/usr/local/bin/innerword");
        assert_eq!(quote("inner-word"), "inner-word");
    }

    #[test]
    fn a_path_with_a_space_is_one_word() {
        assert_eq!(
            quote("/Applications/Inner Word.app/Contents/MacOS/innerword"),
            "'/Applications/Inner Word.app/Contents/MacOS/innerword'"
        );
        assert_eq!(quote(""), "''");
    }

    /// The real shell gives back exactly the text, whatever is in it.
    #[test]
    fn the_shell_reads_back_what_was_quoted() {
        for text in [
            "/Applications/Inner Word.app/Contents/MacOS/innerword",
            "/Users/o'brien/Apps/Inner Word.app/Contents/MacOS/innerword",
            "a \"double\" and a $HOME and a `tick` and a \\ and a * and a ;",
            "tab\there",
            "''",
            "-n",
        ] {
            let output = Command::new("/bin/sh")
                .arg("-c")
                .arg(format!("printf '%s' {}", quote(text)))
                .output()
                .expect("run /bin/sh");
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                text,
                "quoted as {}",
                quote(text)
            );
        }
    }

    #[test]
    fn string_literals_escape_quotes_and_backslashes() {
        assert_eq!(string_literal("/a b/c"), "\"/a b/c\"");
        assert_eq!(string_literal("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }
}
