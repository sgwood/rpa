#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|_| {
            tauri::async_runtime::spawn(async {
                let result = async {
                    let (state, paths) = ai_rpa_node::cli::create_state(None, None)?;
                    ai_rpa_node::cli::serve(state, &paths, ai_rpa_node::config::DEFAULT_BIND, None)
                        .await
                }
                .await;
                if let Err(error) = result {
                    eprintln!("AI RPA embedded node stopped: {error:#}");
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running AI Task Console");
}
