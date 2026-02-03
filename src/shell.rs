use std::io;
use anyhow::Result;

pub fn has_shell_integration() -> io::Result<bool> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
    let launcher_path = std::path::Path::new(&home)
        .join(".config")
        .join("cdtree")
        .join("launcher")
        .join("bash")
        .join("cdtree");
    let source_line = format!("source \"{}\"", launcher_path.to_string_lossy());

    let rc_candidates = [".zshrc", ".bashrc"]
        .map(|name| std::path::Path::new(&home).join(name));

    for rc in rc_candidates.iter() {
        if !rc.exists() {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(rc) {
            if content.contains(&source_line) {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

pub fn setup_shell_integration() -> Result<()> {
    let home = std::env::var("HOME").expect("Could not find HOME directory");
    let config_dir = std::path::Path::new(&home)
        .join(".config")
        .join("cdtree")
        .join("launcher")
        .join("bash");

    // Create config directory if it doesn't exist
    if !config_dir.exists() {
        std::fs::create_dir_all(&config_dir)?;
    }

    // create launcher script
    let launcher_path = config_dir.join("cdtree");
    let shell_func = r#"
# cdtree integration
function cdtree() {
    local target
    target=$(command cdtree "$@") && [ -n "$target" ] && cd "$target"
}
"#;
    std::fs::write(&launcher_path, shell_func)?;
    println!("Created launcher script at {:?}", launcher_path);

    // Update shell config
    let zshrc = std::path::Path::new(&home).join(".zshrc");
    let bashrc = std::path::Path::new(&home).join(".bashrc");

    let target_file = if zshrc.exists() {
        zshrc
    } else if bashrc.exists() {
        bashrc
    } else {
        eprintln!("Could not find .zshrc or .bashrc");
        return Ok(());
    };

    let source_line = format!("source \"{}\"", launcher_path.to_string_lossy());

    // Check if already installed
    let content = std::fs::read_to_string(&target_file)?;
    if content.contains(&source_line) {
        println!("Shell integration already exists in {:?}", target_file);
        return Ok(());
    }

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .append(true)
        .open(&target_file)?;

    use std::io::Write;
    writeln!(file, "\n# cdtree")?;
    writeln!(file, "{}", source_line)?;
    println!("Successfully set up shell integration in {:?}", target_file);
    println!("Please restart your shell or run 'source {:?}' to apply changes.", target_file);

    Ok(())
}
