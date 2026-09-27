pub mod events;
pub mod focus;
pub mod hook_install;
pub mod model;
pub mod registry;
pub mod state;
pub mod transcript;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
