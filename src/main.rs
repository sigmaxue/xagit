// Use the library crate instead of compiling `git`/`ui` a second time in
// the binary: the integration tests exercise exactly this code.
use xagit_lib::{git, ui};

/// Printed by `xagit --help`. The full keymap lives in the TUI (`h`) and
/// in README.md; this is only what is useful before starting the program.
const USAGE: &str = "\
xagit - a Magit-style git TUI

Usage: xagit [PATH]
       xagit --help
       xagit --version

  PATH   repository directory to open (default: current directory)

Status buffer keys:
  h / ?      dispatch (all commands)
  n / p      next / previous section
  TAB        expand / collapse section
  s / u      stage / unstage
  c c        commit
  g          refresh
  q          back (does not quit on the status page)
  C-s / C-r  search this buffer (C-s again: next match)
  C-g        clear the search highlight
  C-x C-c    quit

Environment:
  XAGIT_COLOR=truecolor|256|16|never   force a color depth
  XAGIT_THEME=dark|light               choose the color theme
  XAGIT_ASCII=1                        use ASCII glyphs
  NO_COLOR, TERM, COLORTERM            color detection

Full keymap and limitations: see README.md
https://github.com/sigmaxue/xagit
";

/// `xagit --version`; derived from Cargo.toml so the two cannot diverge.
const VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help") {
        print!("{USAGE}");
        return;
    }
    if args.iter().any(|a| a == "--version") {
        println!("{VERSION}");
        return;
    }
    if let Err(e) = ui::app::main_run(args.first().map(String::as_str)) {
        eprintln!("xagit: {}", git::exec::sanitize(&format!("{e:#}")));
        std::process::exit(1);
    }
}
