use std::fs;
use std::path::PathBuf;

fn main() {
    let config_path: PathBuf = dirs::config_dir()
        .expect("Could not determine config directory")
        .join("cdtree")
        .join("config.json.test");

    match fs::write(&config_path, "hello") {
        Ok(_) => println!("Write ok"),
        Err(e) => println!("Write err: {}", e),
    }
}
