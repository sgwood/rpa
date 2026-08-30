fn main() {
    if ai_rpa_node::cli::is_cli_invocation() {
        let runtime = tokio::runtime::Runtime::new().expect("create CLI runtime");
        if let Err(error) = runtime.block_on(ai_rpa_node::cli::run()) {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
    } else {
        ai_rpa_desktop_lib::run();
    }
}
