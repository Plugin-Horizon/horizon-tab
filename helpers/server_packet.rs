// Every play packet the server sends a guest (heard after `native_watch_server_packets`): TAB's
// anti-override. No `self` here, and the call is cheap when nothing matches.
fn tab_server_packet(player: u128, packet: &mut horizon_protocol::packets::play::Clientbound, cancel: &mut bool) {
    crate::game::tab_glue::guard_packet(player, packet, cancel);
}
