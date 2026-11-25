//! The anti-override guard: TAB's `PacketListener`. Another sender's clientbound packets that undo
//! what TAB put on a client are read here (packet ids watched with `watch_outgoing_packets`) and
//! answered with a verdict: let it pass, drop it, or send it with another body. The wire layout is
//! protocol 777 (26.3): `ClientboundPlayerInfoUpdatePacket` and `ClientboundSetPlayerTeamPacket`.
//! Free of game types, so it is tested on byte strings.

use crate::horizon_tab::api;

/// `ClientboundPlayerInfoUpdatePacket` (`horizon_protocol::ids::play::clientbound`).
pub const PLAYER_INFO_UPDATE: i32 = 71;
/// `ClientboundSetPlayerTeamPacket`.
pub const SET_PLAYER_TEAM: i32 = 111;

/// What the guard decides for one packet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Keep,
    Cancel,
    /// The same packet with this body.
    Replace(Vec<u8>),
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let slice = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn varint(&mut self) -> Option<i32> {
        let mut value: u32 = 0;
        for shift in 0..5 {
            let byte = self.u8()?;
            value |= u32::from(byte & 0x7F) << (shift * 7);
            if byte & 0x80 == 0 {
                return Some(value as i32);
            }
        }
        None
    }
    fn string(&mut self) -> Option<String> {
        let len = usize::try_from(self.varint()?).ok()?;
        String::from_utf8(self.take(len)?.to_vec()).ok()
    }
    fn uuid(&mut self) -> Option<u128> {
        Some(u128::from_be_bytes(self.take(16)?.try_into().ok()?))
    }
    /// A network NBT value (nameless root): a text component.
    fn skip_nbt(&mut self) -> Option<()> {
        let kind = self.u8()?;
        if kind == 0 {
            return Some(());
        }
        self.skip_payload(kind, 0)
    }
    fn skip_payload(&mut self, kind: u8, depth: u32) -> Option<()> {
        if depth > 64 {
            return None;
        }
        match kind {
            1 => drop(self.take(1)?),
            2 => drop(self.take(2)?),
            3 | 5 => drop(self.take(4)?),
            4 | 6 => drop(self.take(8)?),
            7 => {
                let n = usize::try_from(i32::from_be_bytes(self.take(4)?.try_into().ok()?)).ok()?;
                self.take(n)?;
            }
            8 => {
                let n = usize::from(u16::from_be_bytes(self.take(2)?.try_into().ok()?));
                self.take(n)?;
            }
            9 => {
                let inner = self.u8()?;
                let n = i32::from_be_bytes(self.take(4)?.try_into().ok()?);
                for _ in 0..n.max(0) {
                    self.skip_payload(inner, depth + 1)?;
                }
            }
            10 => loop {
                let inner = self.u8()?;
                if inner == 0 {
                    break;
                }
                let n = usize::from(u16::from_be_bytes(self.take(2)?.try_into().ok()?));
                self.take(n)?;
                self.skip_payload(inner, depth + 1)?;
            },
            11 | 12 => {
                let n = usize::try_from(i32::from_be_bytes(self.take(4)?.try_into().ok()?)).ok()?;
                let width = if kind == 11 { 4 } else { 8 };
                self.take(n.checked_mul(width)?)?;
            }
            _ => return None,
        }
        Some(())
    }
}

fn write_varint(out: &mut Vec<u8>, mut value: i32) {
    loop {
        if value & !0x7F == 0 {
            out.push(value as u8);
            return;
        }
        out.push((value as u8 & 0x7F) | 0x80);
        value = ((value as u32) >> 7) as i32;
    }
}

/// A `ClientboundSetPlayerTeamPacket` body to `viewer`: dropped when it would move a player TAB
/// put in a team of its own into another team.
pub fn rule_team(viewer: u128, body: &[u8]) -> Verdict {
    match team_members(body) {
        Some((name, players)) if api::guard_team(viewer, &name, &players) => Verdict::Cancel,
        _ => Verdict::Keep,
    }
}

/// The team name and the member entries a team packet carries (methods 0 add, 3 join).
pub fn team_members(body: &[u8]) -> Option<(String, Vec<String>)> {
    let mut r = Reader::new(body);
    let name = r.string()?;
    let method = r.u8()? as i8;
    if !matches!(method, 0 | 3) {
        return None;
    }
    if method == 0 {
        r.skip_nbt()?;
        r.skip_nbt()?;
        r.skip_nbt()?;
        r.varint()?;
        r.varint()?;
        if r.u8()? != 0 {
            r.varint()?;
        }
        r.u8()?;
    }
    let n = usize::try_from(r.varint()?).ok()?;
    let mut players = Vec::with_capacity(n.min(256));
    for _ in 0..n {
        players.push(r.string()?);
    }
    Some((name, players))
}

/// One entry of a player-info update that carries a game mode, a latency or a display name.
#[derive(Debug, PartialEq, Eq)]
struct InfoEntry {
    uuid: u128,
    game_mode: Option<(usize, usize, i32)>,
    latency: Option<(usize, usize, i32)>,
    display_name: bool,
}

fn parse_info(body: &[u8]) -> Option<Vec<InfoEntry>> {
    let mut r = Reader::new(body);
    let actions = r.u8()?;
    let n = usize::try_from(r.varint()?).ok()?;
    let mut entries = Vec::with_capacity(n.min(256));
    for _ in 0..n {
        let uuid = r.uuid()?;
        if actions & 1 != 0 {
            r.string()?;
            let props = usize::try_from(r.varint()?).ok()?;
            for _ in 0..props {
                r.string()?;
                r.string()?;
                if r.u8()? != 0 {
                    r.string()?;
                }
            }
        }
        if actions & 2 != 0 && r.u8()? != 0 {
            r.take(16 + 8)?;
            let key = usize::try_from(r.varint()?).ok()?;
            r.take(key)?;
            let sig = usize::try_from(r.varint()?).ok()?;
            r.take(sig)?;
        }
        let mut entry = InfoEntry { uuid, game_mode: None, latency: None, display_name: false };
        if actions & 4 != 0 {
            let start = r.pos;
            let value = r.varint()?;
            entry.game_mode = Some((start, r.pos, value));
        }
        if actions & 8 != 0 {
            r.u8()?;
        }
        if actions & 16 != 0 {
            let start = r.pos;
            let value = r.varint()?;
            entry.latency = Some((start, r.pos, value));
        }
        if actions & 32 != 0 {
            entry.display_name = true;
            if r.u8()? != 0 {
                r.skip_nbt()?;
            }
        }
        if actions & 64 != 0 {
            r.varint()?;
        }
        if actions & 128 != 0 {
            r.u8()?;
        }
        entries.push(entry);
    }
    Some(entries)
}

/// A player-info update to `viewer`: TAB's ping spoof and spectator fix are applied to the
/// latency and game mode it carries. A body that does not read whole is left as it is.
pub fn rule_player_info(viewer: u128, body: &[u8]) -> Verdict {
    let Some(entries) = parse_info(body) else { return Verdict::Keep };
    // (start, end, new value), in packet order.
    let mut edits: Vec<(usize, usize, i32)> = Vec::new();
    for entry in &entries {
        if let Some((start, end, mode)) = entry.game_mode {
            if let Some(shown) = api::guard_game_mode(viewer, entry.uuid, mode) {
                edits.push((start, end, shown));
            }
        }
        if let Some((start, end, latency)) = entry.latency {
            if let Some(shown) = api::guard_latency() {
                if shown != latency {
                    edits.push((start, end, shown));
                }
            }
        }
    }
    if edits.is_empty() {
        return Verdict::Keep;
    }
    edits.sort_by_key(|e| e.0);
    let mut out = Vec::with_capacity(body.len());
    let mut at = 0;
    for (start, end, value) in edits {
        out.extend_from_slice(&body[at..start]);
        write_varint(&mut out, value);
        at = end;
    }
    out.extend_from_slice(&body[at..]);
    Verdict::Replace(out)
}

/// The verdict for any watched packet.
pub fn rule(viewer: u128, packet_id: i32, body: &[u8]) -> Verdict {
    match packet_id {
        PLAYER_INFO_UPDATE => rule_player_info(viewer, body),
        SET_PLAYER_TEAM => rule_team(viewer, body),
        _ => Verdict::Keep,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(out: &mut Vec<u8>, s: &str) {
        write_varint(out, s.len() as i32);
        out.extend_from_slice(s.as_bytes());
    }

    #[test]
    fn varints_read_back() {
        for v in [0, 1, 127, 128, 300, 70000, i32::MAX] {
            let mut b = Vec::new();
            write_varint(&mut b, v);
            assert_eq!(Reader::new(&b).varint(), Some(v));
        }
    }

    #[test]
    fn a_join_packet_lists_its_members() {
        let mut b = Vec::new();
        string(&mut b, "team");
        b.push(3);
        write_varint(&mut b, 2);
        string(&mut b, "Alice");
        string(&mut b, "Bob");
        assert_eq!(
            team_members(&b),
            Some(("team".to_owned(), vec!["Alice".to_owned(), "Bob".to_owned()]))
        );
    }

    #[test]
    fn an_add_packet_skips_its_parameters() {
        let mut b = Vec::new();
        string(&mut b, "team");
        b.push(0);
        // display name, prefix, suffix: a string tag (type 8) each, then visibility, collision,
        // an absent colour and the options byte.
        for text in ["", "[", "]"] {
            b.push(8);
            b.extend_from_slice(&(text.len() as u16).to_be_bytes());
            b.extend_from_slice(text.as_bytes());
        }
        write_varint(&mut b, 0);
        write_varint(&mut b, 1);
        b.push(0);
        b.push(3);
        write_varint(&mut b, 1);
        string(&mut b, "Carol");
        assert_eq!(team_members(&b), Some(("team".to_owned(), vec!["Carol".to_owned()])));
    }

    #[test]
    fn a_compound_component_is_skipped() {
        // {text:"a", color:"red"}: compound tag with two string children.
        let mut b = vec![10];
        for (key, value) in [("text", "a"), ("color", "red")] {
            b.push(8);
            b.extend_from_slice(&(key.len() as u16).to_be_bytes());
            b.extend_from_slice(key.as_bytes());
            b.extend_from_slice(&(value.len() as u16).to_be_bytes());
            b.extend_from_slice(value.as_bytes());
        }
        b.push(0);
        b.push(0x2A);
        let mut r = Reader::new(&b);
        assert!(r.skip_nbt().is_some());
        assert_eq!(r.u8(), Some(0x2A));
    }

    #[test]
    fn a_game_mode_and_latency_entry_is_located() {
        let mut b = vec![4 | 16];
        write_varint(&mut b, 1);
        b.extend_from_slice(&7u128.to_be_bytes());
        write_varint(&mut b, 3);
        write_varint(&mut b, 300);
        let entries = parse_info(&b).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].uuid, 7);
        assert_eq!(entries[0].game_mode.map(|g| g.2), Some(3));
        assert_eq!(entries[0].latency.map(|l| l.2), Some(300));
    }

    #[test]
    fn a_truncated_body_is_kept() {
        assert_eq!(rule_player_info(1, &[4, 1, 0, 0]), Verdict::Keep);
        assert_eq!(rule_team(1, &[]), Verdict::Keep);
    }
}
