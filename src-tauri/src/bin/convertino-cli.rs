//! The `convertino` command: a console program, so it can print to the
//! terminal (the app itself is a windowed program on Windows). See cli.rs.

fn main() {
    std::process::exit(convertino_lib::cli_main())
}
