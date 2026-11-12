//! The two expansions every server has: `player` and `server`. Written from the placeholder names
//! users know (`%player_name%`, `%server_online%`, `%server_tps_1_colored%`...); no PlaceholderAPI
//! or expansion source was read. Values the engine does not track (a player's biome, a server's
//! unique joins) are not answered, so they stay as written.

use super::{Expansion, Player, Server};
use std::sync::Arc;

pub(super) fn expansions() -> Vec<Arc<dyn Expansion>> {
    vec![Arc::new(PlayerExpansion), Arc::new(ServerExpansion)]
}

pub struct PlayerExpansion;
pub struct ServerExpansion;

fn game_mode_name(mode: i32) -> &'static str {
    match mode {
        1 => "CREATIVE",
        2 => "ADVENTURE",
        3 => "SPECTATOR",
        _ => "SURVIVAL",
    }
}

/// Java's `String.valueOf(double)` for the usual cases.
pub fn java_double(v: f64) -> String {
    if v.is_finite() && v.fract() == 0.0 && v.abs() < 1e7 {
        format!("{v:.1}")
    } else {
        let s = format!("{v}");
        s
    }
}

/// Two decimals, trailing zeros dropped (`DecimalFormat("#.##")`).
pub fn two_decimals(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" {
        "0".into()
    } else {
        s.to_owned()
    }
}

/// `1d 2h 3m 4s`, leaving out leading zero units.
pub fn duration(secs: i64) -> String {
    let secs = secs.max(0);
    let (d, h, m, s) = (secs / 86400, secs % 86400 / 3600, secs % 3600 / 60, secs % 60);
    let mut parts = Vec::new();
    if d > 0 {
        parts.push(format!("{d}d"));
    }
    if d > 0 || h > 0 {
        parts.push(format!("{h}h"));
    }
    if d > 0 || h > 0 || m > 0 {
        parts.push(format!("{m}m"));
    }
    parts.push(format!("{s}s"));
    parts.join(" ")
}

/// TPS with a colour: green from 18, yellow from 16, red below (and `*` above the rate cap).
pub fn colored_tps(tps: f64) -> String {
    let color = if tps >= 18.0 {
        "&a"
    } else if tps >= 16.0 {
        "&e"
    } else {
        "&c"
    };
    let star = if tps > 20.0 { "*" } else { "" };
    format!("{color}{star}{}", two_decimals(tps.min(20.0)))
}

fn direction(yaw: f32) -> &'static str {
    let y = (yaw.rem_euclid(360.0) + 22.5) % 360.0;
    ["S", "SW", "W", "NW", "N", "NE", "E", "SE"][(y / 45.0) as usize % 8]
}

impl Expansion for PlayerExpansion {
    fn identifier(&self) -> &str {
        "player"
    }
    fn author(&self) -> &str {
        "horizon-tab"
    }
    fn request(&self, player: Option<&Player>, params: &str, _server: &Server) -> Option<String> {
        let p = player?;
        let [x, y, z] = p.position;
        Some(match params.to_ascii_lowercase().as_str() {
            "name" => p.name.clone(),
            "displayname" | "display_name" => p.display_name.clone().unwrap_or_else(|| p.name.clone()),
            "uuid" => p.uuid_string(),
            "world" => p.world.clone(),
            "world_type" | "dimension" => p.dimension.clone(),
            "x" => format!("{}", x.floor() as i64),
            "y" => format!("{}", y.floor() as i64),
            "z" => format!("{}", z.floor() as i64),
            "x_exact" | "exact_x" => java_double(x),
            "y_exact" | "exact_y" => java_double(y),
            "z_exact" | "exact_z" => java_double(z),
            "yaw" => java_double(p.yaw as f64),
            "pitch" => java_double(p.pitch as f64),
            "direction" => direction(p.yaw).to_owned(),
            "health" => two_decimals(p.health as f64),
            "health_rounded" => format!("{}", (p.health as f64).round() as i64),
            "max_health" => "20".into(),
            "ping" => p.ping.to_string(),
            "gamemode" => game_mode_name(p.game_mode).to_owned(),
            "is_op" => (p.op_level > 0).to_string(),
            "online" | "is_online" => "true".into(),
            "allow_flight" => matches!(p.game_mode, 1 | 3).to_string(),
            "is_flying" => (p.game_mode == 3).to_string(),
            "time_since_join" | "online_time" => duration((_server.now_ms - p.joined_at_ms) / 1000),
            _ => return None,
        })
    }
}

impl Expansion for ServerExpansion {
    fn identifier(&self) -> &str {
        "server"
    }
    fn author(&self) -> &str {
        "horizon-tab"
    }
    fn request(&self, _player: Option<&Player>, params: &str, s: &Server) -> Option<String> {
        let lower = params.to_ascii_lowercase();
        if let Some(fmt) = params.strip_prefix("time_") {
            let now = chrono::Local::now().fixed_offset();
            return Some(crate::datefmt::format(fmt, &now));
        }
        Some(match lower.as_str() {
            "online" => s.online.to_string(),
            "max_players" => s.max_players.to_string(),
            "version" => s.version.clone(),
            "name" => s.level_name.clone(),
            "tps" | "tps_1" | "tps_5" | "tps_15" => two_decimals(s.tps.min(20.0)),
            "tps_colored" | "tps_1_colored" | "tps_5_colored" | "tps_15_colored" => colored_tps(s.tps),
            "mspt" => two_decimals(s.mspt),
            "uptime" => duration((s.now_ms - s.started_ms) / 1000),
            "ram_used" => s.memory_used_mb.to_string(),
            "ram_max" | "ram_total" => s.memory_max_mb.to_string(),
            "ram_free" => s.memory_max_mb.saturating_sub(s.memory_used_mb).to_string(),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(two_decimals(19.987), "19.99");
        assert_eq!(two_decimals(20.0), "20");
        assert_eq!(colored_tps(19.5), "&a19.5");
        assert_eq!(colored_tps(15.0), "&c15");
        assert_eq!(duration(3725), "1h 2m 5s");
        assert_eq!(duration(5), "5s");
        assert_eq!(direction(180.0), "N");
    }

    #[test]
    fn player_values() {
        let p = Player {
            name: "Alex".into(),
            position: [1.7, 64.0, -0.5],
            health: 20.0,
            ping: 7,
            ..Player::default()
        };
        let s = Server::default();
        let e = PlayerExpansion;
        assert_eq!(e.request(Some(&p), "name", &s).unwrap(), "Alex");
        assert_eq!(e.request(Some(&p), "z", &s).unwrap(), "-1");
        assert_eq!(e.request(Some(&p), "health", &s).unwrap(), "20");
        assert_eq!(e.request(Some(&p), "biome", &s), None);
        assert_eq!(e.request(None, "name", &s), None);
    }
}
