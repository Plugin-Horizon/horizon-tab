// A player respawned: forget what their client holds from TAB, so the next tick sends it again
// (whether a respawn clears teams, objectives and boss bars is not settled; resending is harmless).
fn tab_respawned(player: u128) {
    horizon_client::tab::player_respawned(player);
}
