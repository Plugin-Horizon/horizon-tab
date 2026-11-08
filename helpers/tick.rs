// Every tick: the players and the server go to the core, its packets to the players.
fn tab_tick(game: &mut crate::game::Game<'_>) {
    let Some((server, players)) = crate::game::tab_glue::snapshot(game) else {
        return;
    };
    crate::game::tab_glue::ensure_started();
    // Until a mod asks, the server neither decodes nor copies the packets it sends; idempotent.
    game.native_watch_server_packets();
    let uuids: Vec<u128> = players.iter().map(|p| p.uuid).collect();
    let packets = horizon_client::tab::tick(crate::game::tab_glue::now_ms(), server, players);
    crate::game::tab_glue::sync_list_names(game, &uuids);
    crate::game::tab_glue::resolve_permissions(game);
    crate::game::tab_glue::send(game, packets);
}
