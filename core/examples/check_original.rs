//! `cargo run --example check_original -- <original TAB file>...`
//!
//! Parses each original file with the port's own YAML reader and prints a redacted report: the parse
//! result, the key paths the port reads (`config::KEYS_READ`), and the key paths it does not read.
//! Values are never printed. Keys of `users.yml` (player names and UUIDs) are printed as `<user>`.

use horizon_tab::config::KEYS_READ;
use horizon_tab::yaml::{self, Yaml};
use std::path::Path;

/// `*` in a pattern matches one key; a pattern matches a path that starts with it.
fn matches(pattern: &str, path: &[String]) -> bool {
    let pat: Vec<&str> = pattern.split('.').collect();
    pat.len() <= path.len() && pat.iter().zip(path).all(|(p, k)| *p == "*" || p.eq_ignore_ascii_case(k))
}

/// Leaf key paths (a map or list is a leaf when empty or a list).
fn leaves(node: &Yaml, prefix: &mut Vec<String>, redact_depth1: bool, out: &mut Vec<Vec<String>>) {
    match node {
        Yaml::Map(entries) if !entries.is_empty() => {
            for (k, v) in entries {
                let shown = if redact_depth1 && prefix.is_empty() { "<user>".to_owned() } else { k.clone() };
                prefix.push(shown);
                leaves(v, prefix, redact_depth1, out);
                prefix.pop();
            }
        }
        _ => out.push(prefix.clone()),
    }
}

fn main() {
    let mut failed = false;
    for arg in std::env::args().skip(1) {
        let path = Path::new(&arg);
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        println!("== {name}");
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                println!("  cannot read: {}", e.kind());
                failed = true;
                continue;
            }
        };
        let tree = match yaml::parse(&text) {
            Ok(t) => t,
            Err(e) => {
                println!("  parse error: {e}");
                failed = true;
                continue;
            }
        };
        println!("  parse: ok");
        if name != "config.yml" && name != "groups.yml" && name != "users.yml" {
            println!("  (read by key at run time: not listed)");
            continue;
        }
        let mut found = Vec::new();
        leaves(&tree, &mut Vec::new(), name == "users.yml", &mut found);
        let (mut read, mut unread) = (0usize, Vec::new());
        for leaf in &found {
            if KEYS_READ.iter().any(|p| matches(p, leaf)) {
                read += 1;
            } else {
                unread.push(leaf.join("."));
            }
        }
        println!("  leaf keys: {} read, {} not read", read, unread.len());
        for key in unread {
            println!("  not read: {key}");
        }
    }
    if failed {
        std::process::exit(1);
    }
}
