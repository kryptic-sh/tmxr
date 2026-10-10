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
        "drt:",
        0,
        0,
        "[-dr] [-t target-session]"
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
        "dt:",
        0,
        0,
        "[-d] [-t target-pane]"
    ),
    cmd!(
        "capture-pane",
        Some("capturep"),
        "pt:",
        0,
        0,
        "[-p] [-t target-pane]"
    ),
    cmd!("choose-buffer", None, "Z", 0, 0, "[-Z]"),
    cmd!("choose-client", None, "Z", 0, 0, "[-Z]"),
    cmd!(
        "choose-tree",
        None,
        "swZt:",
        0,
        0,
        "[-swZ] [-t target-pane]"
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
        "iI:kp:t:T:",
        0,
        1,
        "[-ik] [-I inputs] [-p prompts] [-T prompt-type] [template]"
    ),
    cmd!(
        "confirm-before",
        Some("confirm"),
        "p:t:",
        1,
        ANY,
        "[-p prompt] command"
    ),
    cmd!("copy-command-line", None, "t:", 0, 0, "[-t target-pane]"),
    cmd!("copy-mode", None, "eMut:", 0, 0, "[-eMu] [-t target-pane]"),
    cmd!(
        "customize-mode",
        None,
        "aF:f:Nt:Z",
        0,
        0,
        "[-aNZ] [-F format] [-f filter] [-t target-pane]"
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
        "as:t:",
        0,
        0,
        "[-a] [-s target-session] [-t target-client]"
    ),
    cmd!(
        "display-menu",
        Some("menu"),
        "b:c:C:H:Os:S:t:T:x:y:",
        1,
        ANY,
        "[-O] [-c target-client] [-C starting-choice] [-t target-pane] [-T title] name key command ..."
    ),
    cmd!(
        "display-message",
        Some("display"),
        "pt:",
        0,
        1,
        "[-p] [-t target-pane] [message]"
    ),
    cmd!("display-panes", Some("displayp"), "", 0, 0, ""),
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
        "bdhl:s:t:v",
        0,
        0,
        "[-bdhv] [-l size] [-s src-pane] [-t dst-pane]"
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
        "at:",
        0,
        0,
        "[-a] [-t target-session]"
    ),
    cmd!(
        "kill-window",
        Some("killw"),
        "at:",
        0,
        0,
        "[-a] [-t target-window]"
    ),
    cmd!("last-pane", Some("lastp"), "t:", 0, 0, "[-t target-window]"),
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
        "adks:t:",
        0,
        0,
        "[-adk] [-s src-window] [-t dst-window]"
    ),
    cmd!("list-buffers", Some("lsb"), "", 0, 0, ""),
    cmd!(
        "list-clients",
        Some("lsc"),
        "t:",
        0,
        0,
        "[-t target-session]"
    ),
    cmd!("list-commands", Some("lscm"), "", 0, 0, ""),
    cmd!(
        "list-keys",
        Some("lsk"),
        "1NT:",
        0,
        1,
        "[-1N] [-T key-table] [key]"
    ),
    cmd!(
        "list-panes",
        Some("lsp"),
        "at:",
        0,
        0,
        "[-a] [-t target-window]"
    ),
    cmd!("list-sessions", Some("ls"), "", 0, 0, ""),
    cmd!(
        "list-windows",
        Some("lsw"),
        "at:",
        0,
        0,
        "[-a] [-t target-session]"
    ),
    cmd!(
        "load-buffer",
        Some("loadb"),
        "b:w",
        1,
        1,
        "[-w] [-b buffer-name] path"
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
        "bdhl:s:t:v",
        0,
        0,
        "[-bdhv] [-l size] [-s src-pane] [-t dst-pane]"
    ),
    cmd!(
        "move-window",
        Some("movew"),
        "dkrs:t:",
        0,
        0,
        "[-dkr] [-s src-window] [-t dst-window]"
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
        "dc:n:s:x:y:",
        0,
        ANY,
        "[-d] [-c start-directory] [-n window-name] [-s session-name] [-x width] [-y height] [command]"
    ),
    cmd!(
        "new-window",
        Some("neww"),
        "adc:n:t:",
        0,
        ANY,
        "[-ad] [-c start-directory] [-n window-name] [-t target-window] [command]"
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
        "b:dpt:",
        0,
        0,
        "[-dp] [-b buffer-name] [-t target-pane]"
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
    cmd!("refresh-client", Some("refresh"), "", 0, 0, ""),
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
        "c:kt:",
        0,
        ANY,
        "[-k] [-c start-directory] [-t target-pane] [command]"
    ),
    cmd!(
        "respawn-window",
        Some("respawnw"),
        "c:kt:",
        0,
        ANY,
        "[-k] [-c start-directory] [-t target-window] [command]"
    ),
    cmd!("resurrect-restore", None, "", 0, 0, ""),
    cmd!("resurrect-save", None, "", 0, 0, ""),
    cmd!(
        "rotate-window",
        Some("rotatew"),
        "Dt:U",
        0,
        0,
        "[-DU] [-t target-window]"
    ),
    cmd!(
        "run-shell",
        Some("run"),
        "bCc:d:t:",
        0,
        1,
        "[-bC] [-c start-directory] [-d delay] [-t target-pane] [shell-command]"
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
        "DLlMmRt:UZ",
        0,
        0,
        "[-DLlMmRUZ] [-t target-pane]"
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
        "lMN:t:X",
        0,
        ANY,
        "[-lMX] [-N repeat-count] [-t target-pane] key ..."
    ),
    cmd!("send-prefix", None, "t:", 0, 0, "[-t target-pane]"),
    cmd!("server-access", None, "adlrw", 0, 1, "[-adlrw] [user]"),
    cmd!(
        "set-buffer",
        Some("setb"),
        "ab:w",
        1,
        1,
        "[-aw] [-b buffer-name] data"
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
        "agost:uw",
        1,
        2,
        "[-agosuw] [-t target] option [value]"
    ),
    cmd!(
        "set-window-option",
        Some("setw"),
        "agot:u",
        1,
        2,
        "[-agou] [-t target-window] option [value]"
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
        "gst:vw",
        0,
        1,
        "[-gsvw] [-t target] [option]"
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
    cmd!("source-file", Some("source"), "q", 0, 1, "[-q] [path]"),
    cmd!(
        "split-window",
        Some("splitw"),
        "bc:dfhl:t:v",
        0,
        ANY,
        "[-bdfhv] [-c start-directory] [-l size] [-t target-pane] [command]"
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
        "dDs:t:U",
        0,
        0,
        "[-dDU] [-s src-pane] [-t dst-pane]"
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
        "lnpt:T:",
        0,
        0,
        "[-lnp] [-t target-session] [-T key-table]"
    ),
    cmd!(
        "unbind-key",
        Some("unbind"),
        "anT:",
        0,
        1,
        "[-an] [-T key-table] key"
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
