//! The commands tmxr understands: names, aliases, flag specs, argument
//! counts. Execution lives in the server; this is the shared vocabulary the
//! CLI, config binds and the command prompt are checked against.

use crate::args::{Args, ArgsError};

/// One command's static description.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    pub name: &'static str,
    pub alias: Option<&'static str>,
    /// getopt-style flag spec, see [`crate::args`].
    pub flags: &'static str,
    pub min_args: usize,
    /// `usize::MAX` when the trailing arguments are a command.
    pub max_args: usize,
    pub usage: &'static str,
}

const ANY: usize = usize::MAX;

macro_rules! cmd {
    ($name:literal, $alias:expr, $flags:literal, $min:expr, $max:expr, $usage:literal) => {
        CommandSpec {
            name: $name,
            alias: $alias,
            flags: $flags,
            min_args: $min,
            max_args: $max,
            usage: $usage,
        }
    };
}

/// Every command, sorted by name.
pub const COMMANDS: &[CommandSpec] = &[
    cmd!(
        "attach-session",
        Some("attach"),
        "c:dEf:rt:x",
        0,
        0,
        "[-dErx] [-c working-directory] [-f flags] [-t target-session]"
    ),
    cmd!(
        "bind-key",
        Some("bind"),
        "nrN:T:",
        1,
        ANY,
        "[-nr] [-N note] [-T key-table] key [command [arguments]]"
    ),
    cmd!(
        "break-pane",
        Some("breakp"),
        "abdF:n:Ps:t:",
        0,
        0,
        "[-abdP] [-F format] [-n window-name] [-s src-pane] [-t dst-window]"
    ),
    cmd!(
        "capture-pane",
        Some("capturep"),
        "b:CE:JNpqS:t:",
        0,
        0,
        "[-CJNpq] [-b buffer-name] [-E end-line] [-S start-line] [-t target-pane]"
    ),
    cmd!("choose-buffer", None, "NyZ", 0, 1, "[-NyZ] [template]"),
    cmd!("choose-client", None, "NyZ", 0, 1, "[-NyZ] [template]"),
    cmd!(
        "choose-tree",
        None,
        "NswyZt:",
        0,
        1,
        "[-NswyZ] [-t target-pane] [template]"
    ),
    cmd!(
        "clear-history",
        Some("clearhist"),
        "Ht:",
        0,
        0,
        "[-H] [-t target-pane]"
    ),
    cmd!(
        "clear-prompt-history",
        Some("clearphist"),
        "T:",
        0,
        0,
        "[-T prompt-type]"
    ),
    cmd!("clock-mode", None, "t:", 0, 0, "[-t target-pane]"),
    cmd!(
        "command-prompt",
        None,
        "1bFiI:klNp:t:T:",
        0,
        1,
        "[-1bFiklN] [-I inputs] [-p prompts] [-T prompt-type] [template]"
    ),
    cmd!(
        "confirm-before",
        Some("confirm"),
        "bc:p:t:y",
        1,
        ANY,
        "[-by] [-c confirm-key] [-p prompt] command"
    ),
    cmd!("copy-command-line", None, "t:", 0, 0, "[-t target-pane]"),
    cmd!(
        "copy-mode",
        None,
        "deHMqSs:t:u",
        0,
        0,
        "[-deHMqSu] [-s src-pane] [-t target-pane]"
    ),
    cmd!(
        "customize-mode",
        None,
        "f:Nt:Z",
        0,
        0,
        "[-NZ] [-f filter] [-t target-pane]"
    ),
    cmd!(
        "delete-buffer",
        Some("deleteb"),
        "b:",
        0,
        0,
        "[-b buffer-name]"
    ),
    cmd!(
        "detach-client",
        Some("detach"),
        "aE:Ps:t:",
        0,
        0,
        "[-aP] [-E shell-command] [-s target-session] [-t target-client]"
    ),
    cmd!(
        "display-menu",
        Some("menu"),
        "b:c:C:H:MOs:S:t:T:x:y:",
        1,
        ANY,
        "[-MO] [-c target-client] [-C starting-choice] [-t target-pane] [-T title] name key command ..."
    ),
    cmd!(
        "display-message",
        Some("display"),
        "Cc:d:F:lNpt:",
        0,
        1,
        "[-ClNp] [-c target-client] [-d delay] [-F format] [-t target-pane] [message]"
    ),
    cmd!(
        "display-panes",
        Some("displayp"),
        "bd:N",
        0,
        1,
        "[-bN] [-d duration] [template]"
    ),
    cmd!(
        "display-popup",
        Some("popup"),
        "Bb:Cc:d:e:Eh:kNs:S:t:T:w:x:y:",
        0,
        ANY,
        "[-BCEkN] [-c target-client] [-d start-directory] [-e environment] [-h height] [-t target-pane] [-T title] [-w width] [-x position] [-y position] [shell-command]"
    ),
    cmd!(
        "find-window",
        Some("findw"),
        "CiNrt:TZ",
        1,
        1,
        "[-CiNrTZ] [-t target-pane] match-string"
    ),
    cmd!(
        "has-session",
        Some("has"),
        "t:",
        0,
        0,
        "[-t target-session]"
    ),
    cmd!(
        "if-shell",
        Some("if"),
        "bFt:",
        2,
        3,
        "[-bF] [-t target-pane] shell-command command [command]"
    ),
    cmd!(
        "join-pane",
        Some("joinp"),
        "bdfhl:s:t:v",
        0,
        0,
        "[-bdfhv] [-l size] [-s src-pane] [-t dst-pane]"
    ),
    cmd!(
        "kill-pane",
        Some("killp"),
        "at:",
        0,
        0,
        "[-a] [-t target-pane]"
    ),
    cmd!("kill-server", None, "", 0, 0, ""),
    cmd!(
        "kill-session",
        None,
        "aCt:",
        0,
        0,
        "[-aC] [-t target-session]"
    ),
    cmd!(
        "kill-window",
        Some("killw"),
        "at:",
        0,
        0,
        "[-a] [-t target-window]"
    ),
    cmd!(
        "last-pane",
        Some("lastp"),
        "det:Z",
        0,
        0,
        "[-deZ] [-t target-window]"
    ),
    cmd!(
        "last-window",
        Some("last"),
        "t:",
        0,
        0,
        "[-t target-session]"
    ),
    cmd!(
        "link-window",
        Some("linkw"),
        "abdks:t:",
        0,
        0,
        "[-abdk] [-s src-window] [-t dst-window]"
    ),
    cmd!(
        "list-buffers",
        Some("lsb"),
        "F:f:",
        0,
        0,
        "[-F format] [-f filter]"
    ),
    cmd!(
        "list-clients",
        Some("lsc"),
        "F:f:t:",
        0,
        0,
        "[-F format] [-f filter] [-t target-session]"
    ),
    cmd!("list-commands", Some("lscm"), "", 0, 0, ""),
    cmd!(
        "list-keys",
        Some("lsk"),
        "1aNP:T:",
        0,
        1,
        "[-1aN] [-P prefix-string] [-T key-table] [key]"
    ),
    cmd!(
        "list-panes",
        Some("lsp"),
        "asF:f:t:",
        0,
        0,
        "[-as] [-F format] [-f filter] [-t target-window]"
    ),
    cmd!(
        "list-sessions",
        Some("ls"),
        "F:f:",
        0,
        0,
        "[-F format] [-f filter]"
    ),
    cmd!(
        "list-windows",
        Some("lsw"),
        "aF:f:t:",
        0,
        0,
        "[-a] [-F format] [-f filter] [-t target-session]"
    ),
    cmd!(
        "load-buffer",
        Some("loadb"),
        "b:t:w",
        1,
        1,
        "[-w] [-b buffer-name] [-t target-client] path"
    ),
    cmd!(
        "lock-client",
        Some("lockc"),
        "t:",
        0,
        0,
        "[-t target-client]"
    ),
    cmd!("lock-server", Some("lock"), "", 0, 0, ""),
    cmd!(
        "lock-session",
        Some("locks"),
        "t:",
        0,
        0,
        "[-t target-session]"
    ),
    cmd!(
        "move-pane",
        Some("movep"),
        "bdfhl:s:t:v",
        0,
        0,
        "[-bdfhv] [-l size] [-s src-pane] [-t dst-pane]"
    ),
    cmd!(
        "move-window",
        Some("movew"),
        "abdkrs:t:",
        0,
        0,
        "[-abdkr] [-s src-window] [-t dst-window]"
    ),
    cmd!(
        "navigate-pane",
        None,
        "DLlRt:U",
        0,
        0,
        "[-DLlRU] [-t target-pane]"
    ),
    cmd!(
        "new-session",
        Some("new"),
        "Ac:dDe:EF:f:n:Ps:Xx:y:",
        0,
        ANY,
        "[-AdDEPX] [-c start-directory] [-e environment] [-F format] [-f flags] [-n window-name] [-s session-name] [-x width] [-y height] [command]"
    ),
    cmd!(
        "new-window",
        Some("neww"),
        "abdc:e:F:kn:PSt:",
        0,
        ANY,
        "[-abdkPS] [-c start-directory] [-e environment] [-F format] [-n window-name] [-t target-window] [command]"
    ),
    cmd!(
        "next-layout",
        Some("nextl"),
        "t:",
        0,
        0,
        "[-t target-window]"
    ),
    cmd!(
        "next-window",
        Some("next"),
        "at:",
        0,
        0,
        "[-a] [-t target-session]"
    ),
    cmd!(
        "paste-buffer",
        Some("pasteb"),
        "b:dprs:t:",
        0,
        0,
        "[-dpr] [-s separator] [-b buffer-name] [-t target-pane]"
    ),
    cmd!(
        "pipe-pane",
        Some("pipep"),
        "IOot:",
        0,
        1,
        "[-IOo] [-t target-pane] [shell-command]"
    ),
    cmd!(
        "previous-layout",
        Some("prevl"),
        "t:",
        0,
        0,
        "[-t target-window]"
    ),
    cmd!(
        "previous-window",
        Some("prev"),
        "at:",
        0,
        0,
        "[-a] [-t target-session]"
    ),
    cmd!(
        "refresh-client",
        Some("refresh"),
        "cDf:F:LRU",
        0,
        1,
        "[-cDLRU] [-f flags] [adjustment]"
    ),
    cmd!(
        "rename-session",
        Some("rename"),
        "t:",
        1,
        1,
        "[-t target-session] new-name"
    ),
    cmd!(
        "rename-window",
        Some("renamew"),
        "t:",
        1,
        1,
        "[-t target-window] new-name"
    ),
    cmd!(
        "resize-pane",
        Some("resizep"),
        "DLMRt:Ux:y:Z",
        0,
        1,
        "[-DLMRUZ] [-x width] [-y height] [-t target-pane] [adjustment]"
    ),
    cmd!(
        "resize-window",
        Some("resizew"),
        "aADLRt:Ux:y:",
        0,
        1,
        "[-aADLRU] [-x width] [-y height] [-t target-window] [adjustment]"
    ),
    cmd!(
        "respawn-pane",
        Some("respawnp"),
        "c:e:kt:",
        0,
        ANY,
        "[-k] [-c start-directory] [-t target-pane] [command]"
    ),
    cmd!(
        "respawn-window",
        Some("respawnw"),
        "c:e:kt:",
        0,
        ANY,
        "[-k] [-c start-directory] [-t target-window] [command]"
    ),
    cmd!("resurrect-restore", None, "", 0, 0, ""),
    cmd!("resurrect-save", None, "", 0, 0, ""),
    cmd!(
        "rotate-window",
        Some("rotatew"),
        "Dt:UZ",
        0,
        0,
        "[-DUZ] [-t target-window]"
    ),
    cmd!(
        "run-shell",
        Some("run"),
        "bCc:d:Et:",
        0,
        1,
        "[-bCE] [-c start-directory] [-d delay] [-t target-pane] [shell-command]"
    ),
    cmd!(
        "save-buffer",
        Some("saveb"),
        "ab:",
        1,
        1,
        "[-a] [-b buffer-name] path"
    ),
    cmd!(
        "select-layout",
        Some("selectl"),
        "Enopt:",
        0,
        1,
        "[-Enop] [-t target-window] [layout-name]"
    ),
    cmd!(
        "select-pane",
        Some("selectp"),
        "DdeLlMmRt:T:UZ",
        0,
        0,
        "[-DdeLlMmRUZ] [-t target-pane]"
    ),
    cmd!(
        "select-window",
        Some("selectw"),
        "lnpt:",
        0,
        0,
        "[-lnp] [-t target-window]"
    ),
    cmd!(
        "send-keys",
        Some("send"),
        "c:FHKlMN:Rt:X",
        0,
        ANY,
        "[-FHKlMRX] [-c target-client] [-N repeat-count] [-t target-pane] key ..."
    ),
    cmd!("send-prefix", None, "t:", 0, 0, "[-t target-pane]"),
    cmd!("server-access", None, "adlrw", 0, 1, "[-adlrw] [user]"),
    cmd!(
        "set-buffer",
        Some("setb"),
        "ab:n:t:w",
        0,
        1,
        "[-aw] [-b buffer-name] [-n new-buffer-name] [-t target-client] [data]"
    ),
    cmd!(
        "set-environment",
        Some("setenv"),
        "Fghrt:u",
        1,
        2,
        "[-Fghru] [-t target-session] name [value]"
    ),
    cmd!(
        "set-hook",
        None,
        "agpRt:uw",
        1,
        2,
        "[-agpRuw] [-t target-session] hook [command]"
    ),
    cmd!(
        "set-option",
        Some("set"),
        "aFgopqst:uUw",
        1,
        2,
        "[-aFgopqsuUw] [-t target-pane] option [value]"
    ),
    cmd!(
        "set-window-option",
        Some("setw"),
        "aFgoqt:u",
        1,
        2,
        "[-aFgoqu] [-t target-window] option [value]"
    ),
    cmd!("show-buffer", Some("showb"), "b:", 0, 0, "[-b buffer-name]"),
    cmd!(
        "show-environment",
        Some("showenv"),
        "ghst:",
        0,
        1,
        "[-ghs] [-t target-session] [name]"
    ),
    cmd!(
        "show-hooks",
        None,
        "gpt:w",
        0,
        0,
        "[-gpw] [-t target-session]"
    ),
    cmd!("show-messages", Some("showmsgs"), "", 0, 0, ""),
    cmd!(
        "show-options",
        Some("show"),
        "AgHpqst:vw",
        0,
        1,
        "[-AgHpqsvw] [-t target-pane] [option]"
    ),
    cmd!(
        "show-prompt-history",
        Some("showphist"),
        "T:",
        0,
        0,
        "[-T prompt-type]"
    ),
    cmd!(
        "show-window-options",
        Some("showw"),
        "gt:v",
        0,
        1,
        "[-gv] [-t target-window] [option]"
    ),
    cmd!("source-file", Some("source"), "Fnq", 0, 1, "[-Fnq] [path]"),
    cmd!(
        "split-window",
        Some("splitw"),
        "bc:de:F:fhl:p:Pt:vZ",
        0,
        ANY,
        "[-bdfhPvZ] [-c start-directory] [-e environment] [-F format] [-l size] [-p percentage] [-t target-pane] [command]"
    ),
    cmd!("start-server", Some("start"), "", 0, 0, ""),
    cmd!(
        "suspend-client",
        Some("suspendc"),
        "t:",
        0,
        0,
        "[-t target-client]"
    ),
    cmd!(
        "swap-pane",
        Some("swapp"),
        "dDs:t:UZ",
        0,
        0,
        "[-dDUZ] [-s src-pane] [-t dst-pane]"
    ),
    cmd!(
        "swap-window",
        Some("swapw"),
        "ds:t:",
        0,
        0,
        "[-d] [-s src-window] [-t dst-window]"
    ),
    cmd!(
        "switch-client",
        Some("switchc"),
        "c:Elnprt:T:Z",
        0,
        0,
        "[-ElnprZ] [-c target-client] [-t target-session] [-T key-table]"
    ),
    cmd!(
        "unbind-key",
        Some("unbind"),
        "anqT:",
        0,
        1,
        "[-anq] [-T key-table] key"
    ),
    cmd!(
        "unlink-window",
        Some("unlinkw"),
        "kt:",
        0,
        0,
        "[-k] [-t target-window]"
    ),
    cmd!(
        "wait-for",
        Some("wait"),
        "LSU",
        1,
        1,
        "[-L | -S | -U] channel"
    ),
];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    #[error("unknown command: {0}")]
    Unknown(String),
    #[error("ambiguous command: {0}, could be: {1}")]
    Ambiguous(String, String),
    #[error("command {name}: {source}")]
    Args {
        name: &'static str,
        source: ArgsError,
    },
    #[error("command {0}: too few arguments (usage: {0} {1})")]
    TooFew(&'static str, &'static str),
    #[error("command {0}: too many arguments (usage: {0} {1})")]
    TooMany(&'static str, &'static str),
}

/// Find a command by full name, alias, or unique prefix of a name.
pub fn lookup(name: &str) -> Result<&'static CommandSpec, CommandError> {
    if let Some(c) = COMMANDS
        .iter()
        .find(|c| c.name == name || c.alias == Some(name))
    {
        return Ok(c);
    }
    let matches: Vec<_> = COMMANDS
        .iter()
        .filter(|c| c.name.starts_with(name))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one),
        [] => Err(CommandError::Unknown(name.to_owned())),
        many => Err(CommandError::Ambiguous(
            name.to_owned(),
            many.iter().map(|c| c.name).collect::<Vec<_>>().join(", "),
        )),
    }
}

/// A command ready to run: its spec and parsed arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub spec: &'static CommandSpec,
    pub args: Args,
}

impl Parsed {
    pub fn name(&self) -> &'static str {
        self.spec.name
    }
}

/// Whether a command line (commands split by `;` words) reads the client's
/// standard input: a `load-buffer -` in it, which the client must send
/// ahead.
pub fn reads_stdin(argv: &[String]) -> bool {
    argv.split(|w| w == ";").any(|cmd| {
        parse(cmd).is_ok_and(|p| {
            p.name() == "load-buffer" && p.args.positional().first().is_some_and(|f| f == "-")
        })
    })
}

/// Arguments already split into words, separated into commands as tmux's
/// cmd_parse_from_arguments: a word ending in `;` ends its command (the
/// `;` dropped, and the word too if nothing is left), unless the `;` follows
/// a backslash, which makes the word end in a literal `;` instead.
pub fn split_arguments(words: &[String]) -> Vec<Vec<String>> {
    let mut commands = Vec::new();
    let mut command = Vec::new();
    for word in words {
        let Some(rest) = word.strip_suffix(';') else {
            command.push(word.clone());
            continue;
        };
        if let Some(literal) = rest.strip_suffix('\\') {
            command.push(format!("{literal};"));
            continue;
        }
        if !rest.is_empty() {
            command.push(rest.to_owned());
        }
        commands.push(std::mem::take(&mut command));
    }
    commands.push(command);
    commands.retain(|c| !c.is_empty());
    commands
}

/// `line` as tmux's cmd_list_print gives a command list back, escaped:
/// each command by its full name and its arguments (args_print), joined by
/// ` \; `. `$VAR` is kept as written (tmux has expanded it already), and a
/// `{ … }` block is printed as the string tmxr keeps it as. `None` when the
/// line does not parse.
pub fn print_command_list(line: &str) -> Option<String> {
    let keep = |name: &str| Some(format!("${{{name}}}"));
    let commands = crate::tokenize(line, &keep).ok()?;
    let printed: Option<Vec<String>> = commands
        .iter()
        .map(|words| {
            let p = parse(words).ok()?;
            let args = p.args.print();
            Some(if args.is_empty() {
                p.name().to_owned()
            } else {
                format!("{} {args}", p.name())
            })
        })
        .collect();
    Some(printed?.join(" \\; "))
}

/// Resolve `argv[0]` and parse the rest against its flag spec.
pub fn parse(argv: &[String]) -> Result<Parsed, CommandError> {
    let (name, rest) = argv
        .split_first()
        .ok_or_else(|| CommandError::Unknown(String::new()))?;
    let spec = lookup(name)?;
    let args = Args::parse(spec.flags, rest).map_err(|source| CommandError::Args {
        name: spec.name,
        source,
    })?;
    let n = args.positional().len();
    if n < spec.min_args {
        return Err(CommandError::TooFew(spec.name, spec.usage));
    }
    if n > spec.max_args {
        return Err(CommandError::TooMany(spec.name, spec.usage));
    }
    Ok(Parsed { spec, args })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_split_into_commands_as_tmux() {
        let w = |s: &[&str]| s.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
        let split = |s: &[&str]| split_arguments(&w(s));
        assert_eq!(split(&["a", "b"]), vec![w(&["a", "b"])]);
        assert_eq!(split(&["a", ";", "b"]), vec![w(&["a"]), w(&["b"])]);
        assert_eq!(split(&["a;", "b"]), vec![w(&["a"]), w(&["b"])]);
        // An escaped `;` stays, as the literal word `;` or a word's end.
        assert_eq!(
            split(&["bind", "x", "A", "\\;", "B"]),
            vec![w(&["bind", "x", "A", ";", "B"])]
        );
        assert_eq!(split(&["say", "hi\\;"]), vec![w(&["say", "hi;"])]);
        assert_eq!(split(&[";", ";"]), Vec::<Vec<String>>::new());
    }

    #[test]
    fn command_lists_print_back_as_tmux_prints_them() {
        // tmux 3.6's list-keys for its own copy-mode binds (WSL, 2026-10-20).
        for (given, printed) in [
            (
                "send -FX search-backward -- '#{copy_cursor_word}'",
                "send-keys -FX search-backward -- \"#{copy_cursor_word}\"",
            ),
            (
                "select-pane; send -X -N 5 scroll-up",
                r"select-pane \; send-keys -X -N 5 scroll-up",
            ),
            ("send -X cancel", "send-keys -X cancel"),
        ] {
            assert_eq!(
                print_command_list(given).as_deref(),
                Some(printed),
                "{given}"
            );
        }
        assert_eq!(print_command_list("no-such-command"), None);
    }

    fn v(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn table_is_sorted_unique_and_specs_are_well_formed() {
        for pair in COMMANDS.windows(2) {
            assert!(
                pair[0].name < pair[1].name,
                "{} / {}",
                pair[0].name,
                pair[1].name
            );
        }
        let mut aliases: Vec<_> = COMMANDS.iter().filter_map(|c| c.alias).collect();
        aliases.sort_unstable();
        aliases.dedup();
        assert_eq!(
            aliases.len(),
            COMMANDS.iter().filter(|c| c.alias.is_some()).count()
        );
        for c in COMMANDS {
            assert!(c.min_args <= c.max_args, "{}", c.name);
            assert!(!c.flags.starts_with(':'), "{}", c.name);
        }
    }

    #[test]
    fn load_buffer_dash_reads_stdin() {
        let words = |s: &str| s.split(' ').map(str::to_owned).collect::<Vec<_>>();
        assert!(reads_stdin(&words("load-buffer -")));
        assert!(reads_stdin(&words("loadb -b x -")));
        assert!(reads_stdin(&words("ls ; loadb -")));
        assert!(!reads_stdin(&words("load-buffer file")));
        assert!(!reads_stdin(&words("save-buffer -")));
        assert!(!reads_stdin(&words("send-keys -")));
    }

    #[test]
    fn lookup_by_name_alias_and_prefix() {
        assert_eq!(lookup("split-window").unwrap().name, "split-window");
        assert_eq!(lookup("splitw").unwrap().name, "split-window");
        assert_eq!(lookup("ls").unwrap().name, "list-sessions");
        assert_eq!(lookup("split").unwrap().name, "split-window");
        assert!(matches!(lookup("list"), Err(CommandError::Ambiguous(..))));
        assert!(matches!(
            lookup("frobnicate"),
            Err(CommandError::Unknown(_))
        ));
    }

    #[test]
    fn parse_checks_flags_and_counts() {
        let p = parse(&v(&["splitw", "-h", "-c", "/tmp"])).unwrap();
        assert_eq!(p.name(), "split-window");
        assert!(p.args.has('h'));
        assert_eq!(p.args.value('c'), Some("/tmp"));
        assert!(matches!(
            parse(&v(&["rename-window"])),
            Err(CommandError::TooFew(..))
        ));
        assert!(matches!(
            parse(&v(&["kill-server", "x"])),
            Err(CommandError::TooMany(..))
        ));
        assert!(matches!(
            parse(&v(&["kill-server", "-z"])),
            Err(CommandError::Args { .. })
        ));
    }
}
