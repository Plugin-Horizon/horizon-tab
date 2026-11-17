//! What other mods tell TAB: permissions, permission groups, prefixes and suffixes (a permissions
//! mod such as a LuckPerms port), vanish and Bedrock state. The first registered integration that
//! answers wins. Without any, TAB falls back to the engine: an operator (level 1 and up) has every
//! permission, as on Bukkit; the group is `default`; vanished is the engine's own hidden list.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

pub trait Integration: Send + Sync {
    /// For `/tab debug`.
    fn name(&self) -> &str;
    fn has_permission(&self, _player: u128, _node: &str) -> Option<bool> {
        None
    }
    /// The primary permission group (LuckPerms' `primary-group`).
    fn primary_group(&self, _player: u128) -> Option<String> {
        None
    }
    /// `%luckperms-prefix%` / `%luckperms-suffix%`: the highest prefix or suffix.
    fn prefix(&self, _player: u128) -> Option<String> {
        None
    }
    fn suffix(&self, _player: u128) -> Option<String> {
        None
    }
    /// All prefixes / suffixes joined (`%luckperms-prefixes%`).
    fn prefixes(&self, _player: u128) -> Option<String> {
        None
    }
    fn suffixes(&self, _player: u128) -> Option<String> {
        None
    }
    /// The primary group's weight (`%luckperms-weight%`).
    fn weight(&self, _player: u128) -> Option<i32> {
        None
    }
    fn is_vanished(&self, _player: u128) -> Option<bool> {
        None
    }
    fn is_bedrock(&self, _player: u128) -> Option<bool> {
        None
    }
}

fn list() -> &'static RwLock<Vec<Arc<dyn Integration>>> {
    static L: OnceLock<RwLock<Vec<Arc<dyn Integration>>>> = OnceLock::new();
    L.get_or_init(|| RwLock::new(Vec::new()))
}

/// Adds an integration (after the ones registered before it).
pub fn register(integration: Box<dyn Integration>) {
    list()
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .push(Arc::from(integration));
}

/// Takes the integration with this name out.
pub fn unregister(name: &str) -> bool {
    let mut l = list().write().unwrap_or_else(|e| e.into_inner());
    let before = l.len();
    l.retain(|i| i.name() != name);
    l.len() != before
}

pub fn names() -> Vec<String> {
    list()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|i| i.name().to_owned())
        .collect()
}

pub(crate) fn first<T>(f: impl Fn(&dyn Integration) -> Option<T>) -> Option<T> {
    let l = list().read().unwrap_or_else(|e| e.into_inner());
    l.iter().find_map(|i| f(i.as_ref()))
}

/// The game's permission service (`Game::native_permission_check`) as TAB sees it. The core cannot
/// call the game, so the glue answers the nodes TAB asked about on its next tick; until then, and
/// where nothing answers, the operator fallback stands.
#[derive(Default)]
struct HostPerms {
    /// `None` is "no permissions mod answered" (the operator fallback), asked at the time.
    answers: HashMap<(u128, String), (Option<bool>, i64)>,
    wanted: HashSet<(u128, String)>,
}

fn host() -> &'static Mutex<HostPerms> {
    static H: OnceLock<Mutex<HostPerms>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(HostPerms::default()))
}

/// How long the game's answer for a node is kept before it is asked again.
const HOST_ANSWER_MS: i64 = 5000;

/// Whether any integration or the game's permission service answers permissions (else the
/// operator fallback is used).
pub(crate) fn permission(player: u128, node: &str, op_level: u8) -> bool {
    if let Some(v) = first(|i| i.has_permission(player, node)) {
        return v;
    }
    let mut h = host().lock().unwrap_or_else(|e| e.into_inner());
    let key = (player, node.to_owned());
    match h.answers.get(&key) {
        Some((answer, _)) => answer.unwrap_or(op_level >= 1),
        None => {
            h.wanted.insert(key);
            op_level >= 1
        }
    }
}

/// The nodes the game should be asked about now: those TAB asked and has no answer for, and those
/// whose answer is older than a few seconds.
pub fn host_permission_requests(now_ms: i64) -> Vec<(u128, String)> {
    let mut h = host().lock().unwrap_or_else(|e| e.into_inner());
    let mut out: Vec<(u128, String)> = h.wanted.drain().collect();
    out.extend(
        h.answers
            .iter()
            .filter(|(_, (_, at))| now_ms - *at >= HOST_ANSWER_MS)
            .map(|(k, _)| k.clone()),
    );
    out
}

/// The game's answer for a node: `Some(granted)` from a permissions mod, `None` when none answered.
pub fn host_permission_answer(player: u128, node: &str, answer: Option<bool>, now_ms: i64) {
    let mut h = host().lock().unwrap_or_else(|e| e.into_inner());
    h.wanted.remove(&(player, node.to_owned()));
    h.answers.insert((player, node.to_owned()), (answer, now_ms));
}

/// Forgets the answers for players who are no longer on.
pub fn host_permissions_retain(present: &HashSet<u128>) {
    let mut h = host().lock().unwrap_or_else(|e| e.into_inner());
    h.answers.retain(|(u, _), _| present.contains(u));
    h.wanted.retain(|(u, _)| present.contains(u));
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test: the cache is process-wide, so the steps must not interleave with another test.
    #[test]
    fn host_answers_replace_the_operator_fallback() {
        let (p, node) = (0xfeed_u128, "tab.test.node");
        // Unknown: the fallback answers and the node is wanted.
        assert!(permission(p, node, 4));
        assert!(!permission(p, node, 0));
        assert!(host_permission_requests(0).contains(&(p, node.to_owned())));
        // The game says a mod denies it: even an operator is refused.
        host_permission_answer(p, node, Some(false), 100);
        assert!(!permission(p, node, 4));
        // No mod answered: the fallback again.
        host_permission_answer(p, node, None, 100);
        assert!(permission(p, node, 4));
        assert!(!permission(p, node, 0));
        // Fresh answers are not asked again; old ones are.
        assert!(!host_permission_requests(200).contains(&(p, node.to_owned())));
        assert!(host_permission_requests(100 + HOST_ANSWER_MS).contains(&(p, node.to_owned())));
        // A player who left is forgotten.
        host_permissions_retain(&HashSet::new());
        assert!(!host_permission_requests(10_000).contains(&(p, node.to_owned())));
    }
}
