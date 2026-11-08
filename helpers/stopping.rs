// The server stops (players still connected): take away what TAB showed.
fn tab_stopping(game: &mut crate::game::Game<'_>) {
    let packets = horizon_client::tab::stop();
    crate::game::tab_glue::clear_list_names(game);
    crate::game::tab_glue::send(game, packets);
}
