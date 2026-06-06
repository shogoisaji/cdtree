use anyhow::{Context, Result};
use std::fs;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};

const BEGIN_MARKER: &str = "# >>> cdtree shell integration >>>";
const END_MARKER: &str = "# <<< cdtree shell integration <<<";
const LEGACY_HEADER: &str = "# cdtree";
const LAUNCHER_HEADER: &str = "# cdtree launcher";

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"))
}

fn launcher_path(home: &Path) -> PathBuf {
    home.join(".config")
        .join("cdtree")
        .join("launcher")
        .join("bash")
        .join("cdtree")
}

fn rc_candidates(home: &Path) -> [PathBuf; 2] {
    [home.join(".zshrc"), home.join(".bashrc")]
}

fn source_line(launcher_path: &Path) -> String {
    format!("source {}", shell_quote_path(launcher_path))
}

fn legacy_source_line(launcher_path: &Path) -> String {
    format!("source \"{}\"", launcher_path.to_string_lossy())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn shell_quote_path(path: &Path) -> String {
    shell_quote(&path.to_string_lossy())
}

fn integration_block(launcher_path: &Path) -> String {
    let launcher = shell_quote_path(launcher_path);
    format!(
        r#"{BEGIN_MARKER}
{LAUNCHER_HEADER}
if [ -f {launcher} ]; then
  source {launcher}
fi
{END_MARKER}"#
    )
}

fn print_manual_setup_instructions(target_file: Option<&Path>, launcher_path: &Path, error: &io::Error) {
    if let Some(target_file) = target_file {
        eprintln!(
            "Could not update {:?}: {}.",
            target_file,
            error
        );
        eprintln!(
            "Your shell rc file may be managed by another tool such as Home Manager or Nix."
        );
    } else {
        eprintln!("Could not find .zshrc or .bashrc.");
    }
    eprintln!("\nAdd this block to your shell configuration manually:\n");
    eprintln!("{}", integration_block(launcher_path));
    eprintln!("\nThen reload your shell.");
}

fn write_shell_integration(target_file: &Path, launcher_path: &Path) -> io::Result<bool> {
    let content = fs::read_to_string(target_file)?;
    let cleaned = strip_shell_integration(&content, launcher_path);

    if content.contains(BEGIN_MARKER) && cleaned != content {
        return Ok(false);
    }

    fs::write(target_file, cleaned)?;
    let mut file = fs::OpenOptions::new()
        .read(true)
        .append(true)
        .open(target_file)?;

    writeln!(file, "\n{}", integration_block(launcher_path))?;
    Ok(true)
}

fn is_launcher_if_line(line: &str, launcher_path: &Path) -> bool {
    let launcher = launcher_path.to_string_lossy();
    let quoted_launcher = shell_quote_path(launcher_path);
    let trimmed = line.trim();
    trimmed == format!(r#"if [ -f "{launcher}" ]; then"#)
        || trimmed == format!(r#"if [ -r "{launcher}" ]; then"#)
        || trimmed == format!("if [ -f {quoted_launcher} ]; then")
        || trimmed == format!("if [ -r {quoted_launcher} ]; then")
}

fn is_launcher_source_line(line: &str, launcher_path: &Path) -> bool {
    line.trim() == source_line(launcher_path) || line.trim() == legacy_source_line(launcher_path)
}

fn strip_shell_integration(content: &str, launcher_path: &Path) -> String {
    let mut cleaned = Vec::new();
    let mut in_managed_block = false;
    let mut in_unmanaged_launcher_block = false;
    let mut skipped_legacy_source = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed == BEGIN_MARKER {
            in_managed_block = true;
            continue;
        }

        if in_managed_block {
            if trimmed == END_MARKER {
                in_managed_block = false;
            }
            continue;
        }

        if trimmed == LAUNCHER_HEADER {
            in_unmanaged_launcher_block = true;
            skipped_legacy_source = true;
            continue;
        }

        if in_unmanaged_launcher_block {
            skipped_legacy_source = true;
            if trimmed == "fi" {
                in_unmanaged_launcher_block = false;
            }
            continue;
        }

        if is_launcher_if_line(line, launcher_path) {
            in_unmanaged_launcher_block = true;
            skipped_legacy_source = true;
            continue;
        }

        if is_launcher_source_line(line, launcher_path) {
            skipped_legacy_source = true;
            if cleaned.last().map(|line: &&str| line.trim()) == Some(LEGACY_HEADER) {
                cleaned.pop();
            } else if cleaned.last().map(|line: &&str| line.trim()) == Some(LAUNCHER_HEADER) {
                cleaned.pop();
            }
            continue;
        }

        cleaned.push(line);
    }

    let mut result = cleaned.join("\n");
    if content.ends_with('\n') && !result.is_empty() {
        result.push('\n');
    }

    if skipped_legacy_source {
        while result.contains("\n\n\n") {
            result = result.replace("\n\n\n", "\n\n");
        }
    }

    result
}

pub fn has_shell_integration() -> io::Result<bool> {
    let home = home_dir();
    let launcher_path = launcher_path(&home);
    let source_line = source_line(&launcher_path);
    let legacy_source_line = legacy_source_line(&launcher_path);

    if !launcher_path.exists() {
        return Ok(false);
    }

    for rc in rc_candidates(&home).iter() {
        if !rc.exists() {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(rc) {
            if content.contains(BEGIN_MARKER)
                || content.contains(&source_line)
                || content.contains(&legacy_source_line)
                || content.contains(LAUNCHER_HEADER)
            {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

pub fn setup_shell_integration() -> Result<()> {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .context("Could not find HOME directory")?;
    let launcher_path = launcher_path(&home);
    let config_dir = launcher_path
        .parent()
        .context("Could not resolve launcher directory")?;

    // Create config directory if it doesn't exist
    if !config_dir.exists() {
        std::fs::create_dir_all(&config_dir)?;
    }

    let executable_path = std::env::current_exe().context("Could not resolve cdtree executable")?;
    let quoted_executable = shell_quote_path(&executable_path);

    // create launcher script
    let shell_func = r#"
# cdtree integration
function cdtree() {
    # Handle help and version flags directly without cd
    case "$1" in
        -h|--help|-v|--version|-s|--setup|--uninstall|--doctor)
            __CDTREE_EXE__ "$@"
            return
            ;;
    esac
    local target
    target=$(__CDTREE_EXE__ "$@") || return
    [ -n "$target" ] && [ -d "$target" ] && builtin cd -- "$target"
}
"#
    .replace("__CDTREE_EXE__", &quoted_executable);
    std::fs::write(&launcher_path, shell_func)?;
    println!("Created launcher script at {:?}", launcher_path);

    // Update shell config
    let zshrc = home.join(".zshrc");
    let bashrc = home.join(".bashrc");

    let target_file = if zshrc.exists() {
        zshrc
    } else if bashrc.exists() {
        bashrc
    } else {
        print_manual_setup_instructions(None, &launcher_path, &io::Error::from(io::ErrorKind::NotFound));
        return Ok(());
    };

    match write_shell_integration(&target_file, &launcher_path) {
        Ok(true) => {
            println!("Successfully set up shell integration in {:?}", target_file);
            println!(
                "Please restart your shell or run 'source {:?}' to apply changes.",
                target_file
            );
        }
        Ok(false) => {
            println!("Shell integration already exists in {:?}", target_file);
        }
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => {
            print_manual_setup_instructions(Some(&target_file), &launcher_path, &err);
        }
        Err(err) => return Err(err.into()),
    }

    Ok(())
}

pub fn uninstall_shell_integration() -> Result<()> {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .context("Could not find HOME directory")?;
    let launcher_path = launcher_path(&home);
    let mut changed_rcs = Vec::new();

    for rc in rc_candidates(&home).iter() {
        if !rc.exists() {
            continue;
        }

        let content = fs::read_to_string(&rc)?;
        let cleaned = strip_shell_integration(&content, &launcher_path);
        if cleaned != content {
            fs::write(rc, cleaned)?;
            changed_rcs.push(rc.clone());
        }
    }

    if launcher_path.exists() {
        fs::remove_file(&launcher_path)?;
        println!("Removed launcher script at {:?}", launcher_path);
    }

    if changed_rcs.is_empty() {
        println!("No cdtree shell integration was found in .zshrc or .bashrc.");
    } else {
        for rc in changed_rcs {
            println!("Removed shell integration from {:?}", rc);
        }
        println!("Please restart your shell or reload your shell config.");
    }

    Ok(())
}

pub fn print_shell_integration_status() -> Result<()> {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .context("Could not find HOME directory")?;
    let launcher_path = launcher_path(&home);
    let launcher_exists = launcher_path.exists();
    let mut managed_rcs = Vec::new();
    let mut warning_rcs = Vec::new();
    let mut missing_rcs = Vec::new();
    let mut not_configured_rcs = Vec::new();

    for rc in rc_candidates(&home) {
        if !rc.exists() {
            missing_rcs.push(rc);
            continue;
        }

        let content = fs::read_to_string(&rc)?;
        if content.contains(BEGIN_MARKER) {
            managed_rcs.push(rc);
        } else if content.contains(LAUNCHER_HEADER) {
            warning_rcs.push((rc, "unmanaged guarded launcher block found"));
        } else if content.contains(&source_line(&launcher_path))
            || content.contains(&legacy_source_line(&launcher_path))
        {
            warning_rcs.push((rc, "legacy source line found"));
        } else {
            not_configured_rcs.push(rc);
        }
    }

    let integration_ok = launcher_exists && !managed_rcs.is_empty();

    if integration_ok && warning_rcs.is_empty() {
        println!("cdtree doctor: OK");
        println!();
        println!("[OK] Launcher");
        println!("     {}", launcher_path.display());
        println!();
        println!("[OK] Shell integration");
        for rc in &managed_rcs {
            println!("     {}", rc.display());
        }
        println!();
        println!("No action needed.");
        println!("If cdtree is not active in this terminal, reload your shell.");
    } else {
        println!("cdtree doctor: ACTION NEEDED");
        println!();
        if launcher_exists {
            println!("[OK] Launcher");
        } else {
            println!("[MISS] Launcher");
        }
        println!("       {}", launcher_path.display());

        println!();
        if managed_rcs.is_empty() {
            println!("[MISS] Managed shell integration");
        } else {
            println!("[OK] Managed shell integration");
            for rc in &managed_rcs {
                println!("       {}", rc.display());
            }
        }

        for (rc, reason) in &warning_rcs {
            println!("[WARN] {}: {}", rc.display(), reason);
        }

        for rc in &not_configured_rcs {
            println!("[MISS] {}: not configured", rc.display());
        }

        for rc in &missing_rcs {
            println!("[MISS] {}: missing", rc.display());
        }

        println!();
        if !launcher_exists || managed_rcs.is_empty() {
            println!("Run `cdtree --setup` to repair shell integration.");
        } else if !warning_rcs.is_empty() {
            println!("Run `cdtree --setup` to migrate legacy shell integration.");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{EnvGuard, env_lock};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(prefix: &str) -> Self {
            let mut path = std::env::temp_dir();
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            path.push(format!("{}_{}_{}", prefix, std::process::id(), nanos));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn has_shell_integration_detects_source_line_in_zshrc() {
        let _lock = env_lock();
        let temp = TempDir::new("cdtree_home");
        let _guard = EnvGuard::set("HOME", temp.path.to_str().unwrap());

        let launcher_path = temp
            .path
            .join(".config")
            .join("cdtree")
            .join("launcher")
            .join("bash")
            .join("cdtree");
        let source_line = format!("source \"{}\"", launcher_path.to_string_lossy());

        let zshrc = temp.path.join(".zshrc");
        fs::write(&zshrc, format!("{}\n", source_line)).unwrap();
        fs::create_dir_all(launcher_path.parent().unwrap()).unwrap();
        fs::write(&launcher_path, "# test").unwrap();

        assert!(has_shell_integration().unwrap());
    }

    #[test]
    fn has_shell_integration_checks_bashrc_if_zshrc_missing() {
        let _lock = env_lock();
        let temp = TempDir::new("cdtree_home");
        let _guard = EnvGuard::set("HOME", temp.path.to_str().unwrap());

        let launcher_path = temp
            .path
            .join(".config")
            .join("cdtree")
            .join("launcher")
            .join("bash")
            .join("cdtree");
        let source_line = format!("source \"{}\"", launcher_path.to_string_lossy());

        let bashrc = temp.path.join(".bashrc");
        fs::write(&bashrc, format!("{}\n", source_line)).unwrap();
        fs::create_dir_all(launcher_path.parent().unwrap()).unwrap();
        fs::write(&launcher_path, "# test").unwrap();

        assert!(has_shell_integration().unwrap());
    }

    #[test]
    fn has_shell_integration_requires_launcher_file() {
        let _lock = env_lock();
        let temp = TempDir::new("cdtree_home");
        let _guard = EnvGuard::set("HOME", temp.path.to_str().unwrap());

        let launcher_path = temp
            .path
            .join(".config")
            .join("cdtree")
            .join("launcher")
            .join("bash")
            .join("cdtree");
        let source_line = format!("source \"{}\"", launcher_path.to_string_lossy());

        let zshrc = temp.path.join(".zshrc");
        fs::write(&zshrc, format!("{}\n", source_line)).unwrap();

        assert!(!has_shell_integration().unwrap());
    }

    #[test]
    fn setup_shell_integration_creates_launcher_and_updates_rc() {
        let _lock = env_lock();
        let temp = TempDir::new("cdtree_home");
        let _guard = EnvGuard::set("HOME", temp.path.to_str().unwrap());

        let zshrc = temp.path.join(".zshrc");
        fs::write(&zshrc, "# test\n").unwrap();

        setup_shell_integration().unwrap();

        let launcher_path = temp
            .path
            .join(".config")
            .join("cdtree")
            .join("launcher")
            .join("bash")
            .join("cdtree");
        assert!(launcher_path.exists());
        let launcher_content = fs::read_to_string(&launcher_path).unwrap();
        assert!(launcher_content.contains("target=$("));
        assert!(!launcher_content.contains("command cdtree"));

        let source_line = source_line(&launcher_path);
        let content = fs::read_to_string(&zshrc).unwrap();
        assert!(content.contains(BEGIN_MARKER));
        assert!(content.contains(LAUNCHER_HEADER));
        assert!(content.contains(&source_line));
        assert!(content.contains("[ -f "));
    }

    #[test]
    fn setup_shell_integration_migrates_legacy_source_line() {
        let _lock = env_lock();
        let temp = TempDir::new("cdtree_home");
        let _guard = EnvGuard::set("HOME", temp.path.to_str().unwrap());

        let launcher_path = temp
            .path
            .join(".config")
            .join("cdtree")
            .join("launcher")
            .join("bash")
            .join("cdtree");
        let source_line = source_line(&launcher_path);

        let zshrc = temp.path.join(".zshrc");
        fs::write(&zshrc, format!("# test\n# cdtree\n{}\n", source_line)).unwrap();

        setup_shell_integration().unwrap();

        let content = fs::read_to_string(&zshrc).unwrap();
        assert!(content.contains(BEGIN_MARKER));
        assert_eq!(content.matches(&source_line).count(), 1);
        assert!(!content.contains("\n# cdtree\n"));
    }

    #[test]
    fn setup_shell_integration_migrates_unmanaged_guarded_block() {
        let _lock = env_lock();
        let temp = TempDir::new("cdtree_home");
        let _guard = EnvGuard::set("HOME", temp.path.to_str().unwrap());

        let launcher_path = temp
            .path
            .join(".config")
            .join("cdtree")
            .join("launcher")
            .join("bash")
            .join("cdtree");
        let launcher = launcher_path.to_string_lossy();

        let zshrc = temp.path.join(".zshrc");
        fs::write(
            &zshrc,
            format!(
                r#"# test
# cdtree launcher
if [ -f "{launcher}" ]; then
  source "{launcher}"
fi
"#
            ),
        )
        .unwrap();

        setup_shell_integration().unwrap();

        let content = fs::read_to_string(&zshrc).unwrap();
        assert!(content.contains(BEGIN_MARKER));
        assert_eq!(content.matches(LAUNCHER_HEADER).count(), 1);
        assert_eq!(
            content
                .matches(&format!("source {}", shell_quote(&launcher)))
                .count(),
            1
        );
    }

    #[test]
    fn uninstall_shell_integration_removes_managed_block_and_launcher() {
        let _lock = env_lock();
        let temp = TempDir::new("cdtree_home");
        let _guard = EnvGuard::set("HOME", temp.path.to_str().unwrap());

        let zshrc = temp.path.join(".zshrc");
        fs::write(&zshrc, "# test\n").unwrap();

        setup_shell_integration().unwrap();
        uninstall_shell_integration().unwrap();

        let launcher_path = temp
            .path
            .join(".config")
            .join("cdtree")
            .join("launcher")
            .join("bash")
            .join("cdtree");
        let content = fs::read_to_string(&zshrc).unwrap();
        assert!(!launcher_path.exists());
        assert!(!content.contains(BEGIN_MARKER));
        assert!(content.contains("# test"));
    }

    #[cfg(unix)]
    #[test]
    fn setup_shell_integration_treats_permission_denied_as_manual_setup() {
        use std::os::unix::fs::PermissionsExt;

        let _lock = env_lock();
        let temp = TempDir::new("cdtree_home");
        let _guard = EnvGuard::set("HOME", temp.path.to_str().unwrap());

        let zshrc = temp.path.join(".zshrc");
        fs::write(&zshrc, "# managed\n").unwrap();
        let mut permissions = fs::metadata(&zshrc).unwrap().permissions();
        permissions.set_mode(0o444);
        fs::set_permissions(&zshrc, permissions).unwrap();

        let result = setup_shell_integration();

        let mut permissions = fs::metadata(&zshrc).unwrap().permissions();
        permissions.set_mode(0o644);
        fs::set_permissions(&zshrc, permissions).unwrap();

        result.unwrap();

        let launcher_path = temp
            .path
            .join(".config")
            .join("cdtree")
            .join("launcher")
            .join("bash")
            .join("cdtree");
        let content = fs::read_to_string(&zshrc).unwrap();
        assert!(launcher_path.exists());
        assert!(!content.contains(BEGIN_MARKER));
        assert!(content.contains("# managed"));
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("/tmp/it's here"), "'/tmp/it'\\''s here'");
    }
}
