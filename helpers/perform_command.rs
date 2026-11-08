// A player's `/tab ...` or toggle command: the answer lines go back as the command's feedback.
fn tab_perform_command(
    game: &mut crate::game::Game<'_>,
    player: &horizon_client::mod_api::NativeServerPlayer,
    command: &str,
    ran: &mut bool,
) {
    if *ran {
        return;
    }
    crate::game::tab_glue::resolve_command_permissions(game, player.uuid);
    let answer = horizon_client::tab::command(Some((player.uuid, player.permission)), command);
    if !answer.ran {
        return;
    }
    *ran = true;
    for (line, plain) in answer.lines.into_iter().zip(answer.plain) {
        game.native_feedback(horizon_client::text::Text::literal(plain).with_component(line));
    }
}
