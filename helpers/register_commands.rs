// `/tab [arguments]` and the toggle commands (`/sb`) for every player; TAB's permission nodes are
// checked when they run (operators have them all unless a permissions mod answers).
fn tab_register_commands(commands: &mut Vec<horizon_client::mod_api::NativeCommand>) {
    crate::game::tab_glue::ensure_started();
    for (usage, level) in horizon_client::tab::commands() {
        commands.push(if level == 0 {
            horizon_client::mod_api::NativeCommand::new(&usage)
        } else {
            horizon_client::mod_api::NativeCommand::op(&usage, level)
        });
    }
}
