mod settings;

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("running the Tauri app");
}
