#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Root helper mode: `pkexec devsetup --helper apt +id -id`
    if args.get(1).map(String::as_str) == Some("--helper") {
        std::process::exit(devsetup_lib::helper_run(&args[2..]));
    }
    devsetup_lib::run()
}
