// The server starts: write TAB's default files where missing and load config/tab/.
fn tab_start(dedicated: bool) {
    let _ = dedicated;
    crate::game::tab_glue::ensure_started();
}
