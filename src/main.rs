mod git;
mod ui;

fn main() {
    let arg = std::env::args().nth(1);
    if let Err(e) = ui::app::main_run(arg.as_deref()) {
        eprintln!("xagit: {}", git::exec::sanitize(&format!("{e:#}")));
        std::process::exit(1);
    }
}
