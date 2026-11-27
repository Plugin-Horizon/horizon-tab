//! The tab-list packets the layout and the per-world playerlist send, as bytes: protocol 777
//! (26.3) `ClientboundPlayerInfoUpdatePacket` (a one-byte action set, then per entry the fields of
//! the chosen actions in order) and `ClientboundPlayerInfoRemovePacket`. Display names arrive as
//! network NBT, which the game's text boundary writes; this module only places the bytes.

const ADD_PLAYER: u8 = 0x01;
const GAME_MODE: u8 = 0x04;
const LISTED: u8 = 0x08;
const LATENCY: u8 = 0x10;
const DISPLAY_NAME: u8 = 0x20;
const LIST_ORDER: u8 = 0x40;
const HAT: u8 = 0x80;

fn varint(out: &mut Vec<u8>, mut value: i32) {
    loop {
        if value & !0x7F == 0 {
            out.push(value as u8);
            return;
        }
        out.push((value as u8 & 0x7F) | 0x80);
        value = ((value as u32) >> 7) as i32;
    }
}

fn string(out: &mut Vec<u8>, s: &str) {
    varint(out, s.len() as i32);
    out.extend_from_slice(s.as_bytes());
}

/// One fake entry to add.
pub struct Add<'a> {
    pub uuid: u128,
    pub skin: Option<&'a (String, String)>,
    pub latency: i32,
    /// The display name as network NBT; None for no name.
    pub display: Option<Vec<u8>>,
    pub order: i32,
}

/// ADD_PLAYER with an empty profile name (clients from 1.21.2 sort by list order), game mode 0,
/// listed, the latency, the display name, the list order and the hat shown.
pub fn add_body(entries: &[Add<'_>]) -> Vec<u8> {
    let mut b = vec![ADD_PLAYER | GAME_MODE | LISTED | LATENCY | DISPLAY_NAME | LIST_ORDER | HAT];
    varint(&mut b, entries.len() as i32);
    for e in entries {
        b.extend_from_slice(&e.uuid.to_be_bytes());
        string(&mut b, "");
        match e.skin {
            Some((value, signature)) => {
                varint(&mut b, 1);
                string(&mut b, "textures");
                string(&mut b, value);
                if signature.is_empty() {
                    b.push(0);
                } else {
                    b.push(1);
                    string(&mut b, signature);
                }
            }
            None => varint(&mut b, 0),
        }
        varint(&mut b, 0);
        b.push(1);
        varint(&mut b, e.latency);
        match &e.display {
            Some(nbt) => {
                b.push(1);
                b.extend_from_slice(nbt);
            }
            None => b.push(0),
        }
        varint(&mut b, e.order);
        b.push(1);
    }
    b
}

pub fn remove_body(ids: &[u128]) -> Vec<u8> {
    let mut b = Vec::new();
    varint(&mut b, ids.len() as i32);
    for id in ids {
        b.extend_from_slice(&id.to_be_bytes());
    }
    b
}

/// UPDATE_LISTED for the given players.
pub fn listed_body(ids: &[u128], listed: bool) -> Vec<u8> {
    let mut b = vec![LISTED];
    varint(&mut b, ids.len() as i32);
    for id in ids {
        b.extend_from_slice(&id.to_be_bytes());
        b.push(listed as u8);
    }
    b
}

/// UPDATE_DISPLAY_NAME; each name is network NBT.
pub fn names_body(entries: &[(u128, Vec<u8>)]) -> Vec<u8> {
    let mut b = vec![DISPLAY_NAME];
    varint(&mut b, entries.len() as i32);
    for (id, nbt) in entries {
        b.extend_from_slice(&id.to_be_bytes());
        b.push(1);
        b.extend_from_slice(nbt);
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::horizon_tab::guard;

    /// A network NBT string tag (what a plain-text component is).
    fn nbt_text(s: &str) -> Vec<u8> {
        let mut v = vec![8u8];
        v.extend_from_slice(&(s.len() as u16).to_be_bytes());
        v.extend_from_slice(s.as_bytes());
        v
    }

    #[test]
    fn an_added_entry_reads_back_whole() {
        let skin = ("VALUE".to_owned(), "SIG".to_owned());
        let b = add_body(&[
            Add { uuid: 5, skin: Some(&skin), latency: 1000, display: Some(nbt_text("hi")), order: i32::MAX - 5 },
            Add { uuid: 6, skin: None, latency: 7, display: None, order: 1 },
        ]);
        // The guard's reader walks every field of every action, so a whole read proves the layout.
        assert_eq!(guard::added_names(&b), vec![(5, String::new()), (6, String::new())]);
        assert_eq!(guard::display_name_uuids(&b), vec![5, 6]);
        assert_eq!(b[0], 0xFD);
        // uuid, "" name, one property named textures.
        assert_eq!(&b[2..18], &5u128.to_be_bytes());
        assert_eq!(b[18], 0, "an empty profile name");
        assert_eq!(b[19], 1, "one property");
    }

    #[test]
    fn listed_names_and_removals() {
        let mut want = vec![0x08u8, 1];
        want.extend_from_slice(&1u128.to_be_bytes());
        want.push(0);
        assert_eq!(listed_body(&[1], false), want);
        assert_eq!(remove_body(&[2, 3]).len(), 1 + 32);
        let n = names_body(&[(4, nbt_text("x"))]);
        assert_eq!(guard::display_name_uuids(&n), vec![4]);
    }
}
