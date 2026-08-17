use tauri::Manager;

#[cfg(windows)]
mod cad;
mod knowledge;
mod llm;
mod safety;
mod settings;
mod tools;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default().plugin(tauri_plugin_opener::init());

    #[cfg(windows)]
    let builder = builder.invoke_handler(tauri::generate_handler![
        cad::test_cad_connection,
        cad::draw_test_line,
        cad::undo_last_generation,
        cad::sync_session_objects,
        cad::import_selected_objects,
        settings::get_settings,
        settings::save_settings,
        llm::run_agent,
        llm::confirm_tool_call,
    ]);

    #[cfg(not(windows))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        settings::get_settings,
        settings::save_settings,
        llm::run_agent,
        llm::confirm_tool_call,
    ]);

    builder
        .setup(|app| {
            let window = app.get_webview_window("main").unwrap();
            let _ = window.center();
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
