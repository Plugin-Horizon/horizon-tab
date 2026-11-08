// `tab ...` typed on the dedicated server's console (all permissions).
fn tab_console_command(game: &mut crate::game::Game<'_>, command: &str, ran: &mut bool) {
    if *ran {
        return;
    }
    let answer = horizon_client::tab::command(None, command);
    if !answer.ran {
        return;
    }
    *ran = true;
    for (line, plain) in answer.lines.into_iter().zip(answer.plain) {
        game.native_feedback(horizon_client::text::Text::literal(plain).with_component(line));
    }
}
