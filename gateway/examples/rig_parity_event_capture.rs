use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let signature = gateway::parity::old_engine_event_signature();
    let rendered = serde_json::to_string_pretty(&signature)? + "\n";

    if let Some(path) = std::env::args_os().nth(1) {
        let path = PathBuf::from(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, rendered)?;
    } else {
        print!("{rendered}");
    }

    Ok(())
}
