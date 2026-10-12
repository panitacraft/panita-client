#[tauri::command]
fn application_name() -> &'static str {
    launcher_core::application_name()
}

pub fn run() {
    let result = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![application_name])
        .run(tauri::generate_context!());
    if let Err(error) = result {
        eprintln!("Desktop startup failed: {error}");
    }
}
