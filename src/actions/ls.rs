use serde_json;
use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

pub async fn list_containers() -> Result<(), Box<dyn std::error::Error>> {
    println!("CONTAINER ID\tIMAGE\t\tCOMMAND\t\tCREATED\t\tSTATUS\t\tPORTS");

    let containers_dir = "./containers";
    if !fs::metadata(containers_dir).is_ok() {
        println!("No containers found");
        return Ok(());
    }

    let entries = fs::read_dir(containers_dir)?;

    for entry in entries {
        if let Ok(entry) = entry {
            let container_id = entry.file_name().to_string_lossy().to_string();

            let status = Command::new("ip").args(&["netns", "list"]).output()?;

            let netns_output = String::from_utf8_lossy(&status.stdout);
            let is_running = netns_output.contains(&container_id);

            let timestamp_part = container_id.strip_prefix("rustainer_").unwrap_or("0");
            let timestamp = timestamp_part.parse::<u64>().unwrap_or(0);

            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let elapsed_secs = now.saturating_sub(timestamp);
            let created = if elapsed_secs < 60 {
                format!("{}s ago", elapsed_secs)
            } else if elapsed_secs < 3600 {
                format!("{}m ago", elapsed_secs / 60)
            } else if elapsed_secs < 86400 {
                format!("{}h ago", elapsed_secs / 3600)
            } else {
                format!("{}d ago", elapsed_secs / 86400)
            };
            let mut image = "N/A".to_string();
            let mut command = "N/A".to_string();
            let mut ports = "N/A".to_string();

            let metadata_path = format!("{}/{}/metadata.json", containers_dir, container_id);

            let metadata_content = fs::read_to_string(&metadata_path);

            if let Ok(content) = metadata_content {
                if let Ok(metadata) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(img) = metadata.get("image").and_then(|v| v.as_str()) {
                        image = img.to_string();
                    }
                    if let Some(cmd) = metadata.get("command").and_then(|v| v.as_str()) {
                        command = cmd.to_string();
                    }
                    if let Some(port_array) = metadata.get("ports").and_then(|v| v.as_array()) {
                        if !port_array.is_empty() {
                            let port_strs: Vec<String> = port_array
                                .iter()
                                .filter_map(|p| p.as_str().map(String::from))
                                .collect();
                            if !port_strs.is_empty() {
                                ports = port_strs.join(", ");
                            }
                        }
                    }
                }
            }

            let status_str = if is_running { "Up" } else { "Exited" };

            println!(
                "{}\t{}\t{}\t{}\t{}\t{}",
                container_id, image, command, created, status_str, ports
            );
        }
    }

    Ok(())
}
