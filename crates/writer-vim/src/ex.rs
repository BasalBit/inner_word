//! Parsing of `:` command lines.

/// The base of a line address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AddrBase {
    /// A line number, 1-based.
    Number(usize),
    /// `.`: the line of the caret.
    Current,
    /// `$`: the last line.
    Last,
    /// `'x`: the line of mark `x`.
    Mark(char),
}

/// A line address: a base plus `+N` / `-N` offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Addr {
    pub base: AddrBase,
    pub offset: isize,
}

/// The range in front of a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RangeSpec {
    /// No range given.
    None,
    /// `%`: every line.
    All,
    /// One address.
    One(Addr),
    /// `a,b`.
    Two(Addr, Addr),
}

/// A piece of a `:s` replacement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RepPiece {
    /// Literal text.
    Text(String),
    /// The matched text (`&` or `\0`).
    Match,
}

/// The arguments of `:s/pattern/replacement/flags`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SubstArgs {
    /// The raw pattern; empty means "the last search pattern".
    pub pattern: String,
    pub replacement: Vec<RepPiece>,
    /// `g`: every match in a line, not only the first.
    pub global: bool,
    /// `i` (`Some(false)`) or `I` (`Some(true)`): case sensitivity.
    pub case_sensitive: Option<bool>,
    /// `n`: count the matches, change nothing.
    pub count_only: bool,
}

/// A parsed command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExCommand {
    /// Only a range: go to that line.
    Goto,
    Write {
        args: String,
    },
    Quit {
        force: bool,
    },
    WriteQuit,
    Delete {
        register: Option<char>,
    },
    Yank {
        register: Option<char>,
    },
    Substitute(SubstArgs),
    /// `:s`, `:&`, `:&&` without arguments: repeat the last substitution.
    RepeatSubstitute {
        keep_flags: bool,
    },
    NoHighlight,
    Join,
    ShiftRight(usize),
    ShiftLeft(usize),
    Undo,
    Redo,
    /// Not a command of the engine.
    Host {
        name: String,
        args: String,
        bang: bool,
    },
}

/// A parsed command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Parsed {
    pub range: RangeSpec,
    pub command: ExCommand,
    /// The count after `:d`, `:y`, `:j`, `:>` and `:<`: that many lines,
    /// starting with the last line of the range.
    pub count: Option<usize>,
    /// `!` after the command name.
    pub bang: bool,
}

fn parse_addr(s: &str) -> (Option<Addr>, &str) {
    let mut rest = s.trim_start();
    let mut base = None;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 {
        let n = rest[..digits].parse::<usize>().unwrap_or(usize::MAX);
        base = Some(AddrBase::Number(n));
        rest = &rest[digits..];
    } else if let Some(r) = rest.strip_prefix('.') {
        base = Some(AddrBase::Current);
        rest = r;
    } else if let Some(r) = rest.strip_prefix('$') {
        base = Some(AddrBase::Last);
        rest = r;
    } else if let Some(r) = rest.strip_prefix('\'') {
        let mut it = r.chars();
        if let Some(c) = it.next() {
            base = Some(AddrBase::Mark(c));
            rest = it.as_str();
        }
    }
    let mut offset: isize = 0;
    let mut had_offset = false;
    loop {
        let r = rest.trim_start();
        let sign = match r.chars().next() {
            Some('+') => 1,
            Some('-') => -1,
            _ => break,
        };
        let r = &r[1..];
        let digits = r.bytes().take_while(u8::is_ascii_digit).count();
        let n = if digits > 0 {
            r[..digits].parse::<isize>().unwrap_or(isize::MAX / 4)
        } else {
            1
        };
        offset = offset.saturating_add(sign * n);
        had_offset = true;
        rest = &r[digits..];
    }
    match (base, had_offset) {
        (Some(base), _) => (Some(Addr { base, offset }), rest),
        (None, true) => (
            Some(Addr {
                base: AddrBase::Current,
                offset,
            }),
            rest,
        ),
        (None, false) => (None, rest),
    }
}

fn parse_range(s: &str) -> (RangeSpec, &str) {
    let s = s.trim_start();
    if let Some(rest) = s.strip_prefix('%') {
        return (RangeSpec::All, rest);
    }
    let (first, rest) = parse_addr(s);
    let after = rest.trim_start();
    if let Some(r) = after.strip_prefix([',', ';']) {
        let current = Addr {
            base: AddrBase::Current,
            offset: 0,
        };
        let (second, rest) = parse_addr(r);
        return (
            RangeSpec::Two(first.unwrap_or(current), second.unwrap_or(current)),
            rest,
        );
    }
    match first {
        Some(a) => (RangeSpec::One(a), rest),
        None => (RangeSpec::None, rest),
    }
}

/// Whether `name` abbreviates `full` with at least `min` letters.
fn abbreviates(name: &str, full: &str, min: usize) -> bool {
    name.len() >= min && full.starts_with(name)
}

fn parse_replacement(raw: &str) -> Vec<RepPiece> {
    let mut out: Vec<RepPiece> = Vec::new();
    let mut lit = String::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match c {
            '&' => {
                if !lit.is_empty() {
                    out.push(RepPiece::Text(std::mem::take(&mut lit)));
                }
                out.push(RepPiece::Match);
            }
            '\\' => match chars.next() {
                Some('0') => {
                    if !lit.is_empty() {
                        out.push(RepPiece::Text(std::mem::take(&mut lit)));
                    }
                    out.push(RepPiece::Match);
                }
                Some('n' | 'r') => lit.push('\n'),
                Some('t') => lit.push('\t'),
                Some(other) => lit.push(other),
                None => lit.push('\\'),
            },
            c => lit.push(c),
        }
    }
    if !lit.is_empty() {
        out.push(RepPiece::Text(lit));
    }
    out
}

/// Splits `s` at the first unescaped `delim`; the delimiter is dropped.
fn split_delim(s: &str, delim: char) -> (&str, Option<&str>) {
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == delim {
            return (&s[..i], Some(&s[i + c.len_utf8()..]));
        }
    }
    (s, None)
}

fn parse_substitute(args: &str) -> Result<ExCommand, String> {
    let args = args.trim_start();
    let Some(delim) = args.chars().next() else {
        return Ok(ExCommand::RepeatSubstitute { keep_flags: false });
    };
    if delim.is_alphanumeric() || delim.is_whitespace() || matches!(delim, '\\' | '"' | '|') {
        return Err("E146: Regular expressions can't be delimited by letters".into());
    }
    let body = &args[delim.len_utf8()..];
    let (pattern, rest) = split_delim(body, delim);
    let (replacement, flags) = match rest {
        Some(r) => split_delim(r, delim),
        None => ("", None),
    };
    let mut out = SubstArgs {
        pattern: pattern.to_string(),
        replacement: parse_replacement(replacement),
        global: false,
        case_sensitive: None,
        count_only: false,
    };
    for f in flags.unwrap_or("").trim().chars() {
        match f {
            'g' => out.global = !out.global,
            'i' => out.case_sensitive = Some(false),
            'I' => out.case_sensitive = Some(true),
            'n' => out.count_only = true,
            'c' => return Err("The c flag (confirm each match) is not supported".into()),
            'e' | '&' => {}
            c if c.is_ascii_digit() || c == ' ' => {}
            c => return Err(format!("E488: Trailing characters: {c}")),
        }
    }
    Ok(ExCommand::Substitute(out))
}

fn register_arg(args: &str) -> Option<char> {
    args.trim().chars().next().filter(|c| !c.is_ascii_digit())
}

/// Parses a command line (without the leading `:`).
///
/// Supported: a range (`N`, `.`, `$`, `'m`, each with `+N` / `-N`; `a,b`;
/// `%`), and the commands `w[rite]`, `wa[ll]`, `up[date]`, `q[uit]`,
/// `qa[ll]`, `clo[se]`, `wq`, `wqa[ll]`, `x[it]`, `xa[ll]`, `exi[t]`,
/// `d[elete] [x]`, `y[ank] [x]`, `s[ubstitute]/pat/rep/[flags]`, `&`, `&&`,
/// `noh[lsearch]`, `j[oin]`, `>`, `<`, `u[ndo]`, `red[o]`. Anything else
/// that starts with a letter is a host command.
pub(crate) fn parse(line: &str) -> Result<Parsed, String> {
    let line = line.trim_start_matches([':', ' ', '\t']);
    let (range, rest) = parse_range(line);
    let rest = rest.trim_start();
    if rest.is_empty() {
        return Ok(Parsed {
            range,
            command: ExCommand::Goto,
            count: None,
            bang: false,
        });
    }
    let first = rest.chars().next().expect("non-empty");
    if first == '>' || first == '<' {
        let n = rest.chars().take_while(|c| *c == first).count();
        let tail = rest[n..].trim();
        if !tail.is_empty() && !tail.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("E488: Trailing characters: {tail}"));
        }
        let command = if first == '>' {
            ExCommand::ShiftRight(n)
        } else {
            ExCommand::ShiftLeft(n)
        };
        return Ok(Parsed {
            range,
            command,
            count: tail.parse().ok(),
            bang: false,
        });
    }
    if first == '&' {
        let keep_flags = rest.starts_with("&&");
        return Ok(Parsed {
            range,
            command: ExCommand::RepeatSubstitute { keep_flags },
            count: None,
            bang: false,
        });
    }
    if !first.is_alphabetic() {
        return Err(format!("E492: Not an editor command: {rest}"));
    }
    let alpha_len: usize = rest
        .chars()
        .take_while(|c| c.is_alphabetic())
        .map(char::len_utf8)
        .sum();
    let name = &rest[..alpha_len];
    let after = &rest[alpha_len..];
    let (bang, args_raw) = match after.strip_prefix('!') {
        Some(a) => (true, a),
        None => (false, after),
    };
    let args = args_raw.trim();
    // `:d 3`, `:y a 2`, `:j 4`: a trailing number is a count.
    let mut count = None;
    let takes_count = abbreviates(name, "delete", 1)
        || abbreviates(name, "yank", 1)
        || abbreviates(name, "join", 1);
    let args = if takes_count {
        match args.rsplit_once(char::is_whitespace) {
            Some((head, tail)) if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) => {
                count = tail.parse().ok();
                head.trim()
            }
            None if !args.is_empty() && args.bytes().all(|b| b.is_ascii_digit()) => {
                count = args.parse().ok();
                ""
            }
            _ => args,
        }
    } else {
        args
    };
    let command = if abbreviates(name, "substitute", 1) {
        parse_substitute(args_raw)?
    } else if abbreviates(name, "write", 1)
        || abbreviates(name, "wall", 2)
        || abbreviates(name, "update", 2)
    {
        ExCommand::Write {
            args: args.to_string(),
        }
    } else if abbreviates(name, "quit", 1)
        || abbreviates(name, "qall", 2)
        || abbreviates(name, "close", 3)
    {
        ExCommand::Quit { force: bang }
    } else if name == "wq"
        || abbreviates(name, "wqall", 3)
        || abbreviates(name, "xit", 1)
        || abbreviates(name, "xall", 2)
        || abbreviates(name, "exit", 3)
    {
        ExCommand::WriteQuit
    } else if abbreviates(name, "delete", 1) {
        ExCommand::Delete {
            register: register_arg(args),
        }
    } else if abbreviates(name, "yank", 1) {
        ExCommand::Yank {
            register: register_arg(args),
        }
    } else if abbreviates(name, "nohlsearch", 3) {
        ExCommand::NoHighlight
    } else if abbreviates(name, "join", 1) {
        ExCommand::Join
    } else if abbreviates(name, "undo", 1) {
        ExCommand::Undo
    } else if abbreviates(name, "redo", 3) {
        ExCommand::Redo
    } else {
        // A host command: its name may continue with digits, `-` and `_`.
        let ext: usize = after
            .chars()
            .take_while(|c| c.is_alphanumeric() || matches!(c, '-' | '_'))
            .map(char::len_utf8)
            .sum();
        let name = &rest[..alpha_len + ext];
        let after = &rest[alpha_len + ext..];
        let (bang, args) = match after.strip_prefix('!') {
            Some(a) => (true, a),
            None => (false, after),
        };
        ExCommand::Host {
            name: name.to_string(),
            args: args.trim().to_string(),
            bang,
        }
    };
    Ok(Parsed {
        range,
        command,
        count,
        bang,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(s: &str) -> ExCommand {
        parse(s).unwrap().command
    }

    #[test]
    fn ranges() {
        let n = |n| Addr {
            base: AddrBase::Number(n),
            offset: 0,
        };
        assert_eq!(parse("12").unwrap().range, RangeSpec::One(n(12)));
        assert_eq!(parse("%d").unwrap().range, RangeSpec::All);
        assert_eq!(parse("2,5d").unwrap().range, RangeSpec::Two(n(2), n(5)));
        let p = parse(".,$-1 d").unwrap();
        assert_eq!(
            p.range,
            RangeSpec::Two(
                Addr {
                    base: AddrBase::Current,
                    offset: 0
                },
                Addr {
                    base: AddrBase::Last,
                    offset: -1
                }
            )
        );
        assert_eq!(p.command, ExCommand::Delete { register: None });
        let p = parse("'<,'>s/a/b/").unwrap();
        assert!(matches!(
            p.range,
            RangeSpec::Two(
                Addr {
                    base: AddrBase::Mark('<'),
                    ..
                },
                Addr {
                    base: AddrBase::Mark('>'),
                    ..
                }
            )
        ));
        let p = parse("+3").unwrap();
        assert_eq!(
            p.range,
            RangeSpec::One(Addr {
                base: AddrBase::Current,
                offset: 3
            })
        );
        assert_eq!(p.command, ExCommand::Goto);
        assert_eq!(parse("").unwrap().range, RangeSpec::None);
    }

    #[test]
    fn commands() {
        assert_eq!(
            cmd("w"),
            ExCommand::Write {
                args: String::new()
            }
        );
        assert_eq!(
            cmd("write"),
            ExCommand::Write {
                args: String::new()
            }
        );
        assert_eq!(cmd("q"), ExCommand::Quit { force: false });
        assert_eq!(cmd("q!"), ExCommand::Quit { force: true });
        assert_eq!(cmd("wq"), ExCommand::WriteQuit);
        assert_eq!(cmd("x"), ExCommand::WriteQuit);
        assert_eq!(
            cmd("d a"),
            ExCommand::Delete {
                register: Some('a')
            }
        );
        assert_eq!(cmd("y"), ExCommand::Yank { register: None });
        assert_eq!(cmd("noh"), ExCommand::NoHighlight);
        assert_eq!(cmd("nohlsearch"), ExCommand::NoHighlight);
        assert_eq!(cmd("j"), ExCommand::Join);
        assert_eq!(cmd(">>"), ExCommand::ShiftRight(2));
        assert_eq!(cmd("<"), ExCommand::ShiftLeft(1));
        assert_eq!(cmd("u"), ExCommand::Undo);
        assert_eq!(cmd("red"), ExCommand::Redo);
        assert_eq!(cmd("s"), ExCommand::RepeatSubstitute { keep_flags: false });
        assert_eq!(cmd("&&"), ExCommand::RepeatSubstitute { keep_flags: true });
        assert_eq!(
            cmd("export md --draft"),
            ExCommand::Host {
                name: "export".into(),
                args: "md --draft".into(),
                bang: false
            }
        );
        assert_eq!(
            cmd("set-theme! night"),
            ExCommand::Host {
                name: "set-theme".into(),
                args: "night".into(),
                bang: true
            }
        );
        assert!(parse("!ls").is_err());
    }

    #[test]
    fn substitute() {
        let ExCommand::Substitute(s) = cmd("s/foo/bar/g") else {
            panic!()
        };
        assert_eq!(s.pattern, "foo");
        assert_eq!(s.replacement, vec![RepPiece::Text("bar".into())]);
        assert!(s.global && s.case_sensitive.is_none());
        let ExCommand::Substitute(s) = cmd("s#a/b#[&]\\&\\n#gi") else {
            panic!()
        };
        assert_eq!(s.pattern, "a/b");
        assert_eq!(
            s.replacement,
            vec![
                RepPiece::Text("[".into()),
                RepPiece::Match,
                RepPiece::Text("]&\n".into())
            ]
        );
        assert_eq!(s.case_sensitive, Some(false));
        let ExCommand::Substitute(s) = cmd("s/a\\/b/c") else {
            panic!()
        };
        assert_eq!(s.pattern, "a\\/b");
        assert_eq!(s.replacement, vec![RepPiece::Text("c".into())]);
        let ExCommand::Substitute(s) = cmd("s/x//") else {
            panic!()
        };
        assert!(s.replacement.is_empty());
        assert!(parse("s/a/b/c").is_err());
        assert!(parse("s/a/b/z").is_err());
    }
}
