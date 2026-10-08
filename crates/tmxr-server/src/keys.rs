//! Key tables: `root`, `prefix`, `copy-mode-vi` and any user table.

use std::collections::HashMap;

use tmxr_command::BindKey;
use tmxr_config::Config;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindSpec {
    /// Command list in tmux's command language.
    pub cmd: String,
    pub note: Option<String>,
    pub repeat: bool,
}

#[derive(Debug, Default, Clone)]
pub struct KeyTables {
    tables: HashMap<String, HashMap<BindKey, BindSpec>>,
}

impl KeyTables {
    /// Build the tables from the config. Keys that do not parse are skipped
    /// and reported, so one typo does not lose every other bind.
    pub fn from_config(cfg: &Config) -> (Self, Vec<String>) {
        let mut t = Self::default();
        let mut errors = Vec::new();
        for (table, key, bind) in cfg.binds() {
            match key.parse::<BindKey>() {
                Ok(k) => t.bind(
                    table,
                    k,
                    BindSpec {
                        cmd: bind.cmd.clone(),
                        note: bind.note.clone(),
                        repeat: bind.repeat,
                    },
                ),
                Err(e) => errors.push(format!("config [keys.{table}]: {e}")),
            }
        }
        (t, errors)
    }

    pub fn get(&self, table: &str, key: &BindKey) -> Option<&BindSpec> {
        self.tables.get(table)?.get(key)
    }

    pub fn bind(&mut self, table: &str, key: BindKey, spec: BindSpec) {
        self.tables
            .entry(table.to_owned())
            .or_default()
            .insert(key, spec);
    }

    pub fn unbind(&mut self, table: &str, key: &BindKey) -> bool {
        self.tables
            .get_mut(table)
            .is_some_and(|t| t.remove(key).is_some())
    }

    pub fn unbind_all(&mut self, table: &str) {
        self.tables.remove(table);
    }

    /// Every bind, sorted by table then key name.
    pub fn list(&self, table: Option<&str>) -> Vec<(String, BindKey, BindSpec)> {
        let mut out: Vec<_> = self
            .tables
            .iter()
            .filter(|(name, _)| table.is_none_or(|t| t == name.as_str()))
            .flat_map(|(name, keys)| keys.iter().map(move |(k, b)| (name.clone(), *k, b.clone())))
            .collect();
        out.sort_by(|a, b| (&a.0, a.1.to_string()).cmp(&(&b.0, b.1.to_string())));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tables_resolve_the_config_binds() {
        let (t, errors) = KeyTables::from_config(&tmxr_config::defaults());
        assert!(errors.is_empty(), "{errors:?}");
        let k = |s: &str| s.parse::<BindKey>().unwrap();
        assert_eq!(
            t.get("prefix", &k("%")).unwrap().cmd,
            "split-window -h -c \"#{pane_current_path}\""
        );
        assert_eq!(t.get("root", &k("M-l")).unwrap().cmd, "next-window");
        assert!(t.get("prefix", &k("C-Up")).unwrap().repeat);
        assert!(t.get("root", &k("x")).is_none());
        let listed = t.list(Some("copy-mode-vi"));
        assert!(listed.iter().all(|(table, _, _)| table == "copy-mode-vi"));
        assert!(listed.iter().any(|(_, key, _)| key.to_string() == "v"));
    }
}
