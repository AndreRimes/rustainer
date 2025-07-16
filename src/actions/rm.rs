use std::{fs, process::Command};

pub async fn remove_container(container_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let container_dir = format!("./containers/{}", container_id);

    if !fs::metadata(&container_dir).is_ok() {
        return Err(format!("Container {} does not exist", container_id).into());
    }

    stop_container(container_id)?;

    fs::remove_dir_all(&container_dir)?;

    println!("Container {} removed", container_id);

    Ok(())
}

fn stop_container(container_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Stopping container {}", container_id);

    let mut process_to_kill = None;

    let metadata_path = format!("./containers/{}/metadata.json", container_id);
    let mut ports = Vec::new();

    if let Ok(metadata_content) = fs::read_to_string(&metadata_path) {
        if let Ok(metadata) = serde_json::from_str::<serde_json::Value>(&metadata_content) {
            if let Some(port_array) = metadata.get("ports").and_then(|v| v.as_array()) {
                for port in port_array {
                    if let Some(port_str) = port.as_str() {
                        ports.push(port_str.to_string());
                    }
                }
            }
        }
    }

    for port_mapping in &ports {
        let parts: Vec<&str> = port_mapping.split(':').collect();
        if parts.len() == 2 {
            let host_port = parts[0];

            let _ = Command::new("iptables")
                .args(&[
                    "-t",
                    "nat",
                    "-D",
                    "PREROUTING",
                    "-p",
                    "tcp",
                    "--dport",
                    host_port,
                    "-j",
                    "DNAT",
                ])
                .output();

            let _ = Command::new("iptables")
                .args(&[
                    "-t", "nat", "-D", "OUTPUT", "-p", "tcp", "--dport", host_port, "-j", "DNAT",
                ])
                .output();
        }
    }

    let output = Command::new("ip").args(&["netns", "list"]).output()?;

    if String::from_utf8_lossy(&output.stdout).contains(container_id) {
        println!("Killing all processes in container namespace");

        let _ = Command::new("nsenter")
            .args(&[
                "--net=/var/run/netns/",
                container_id,
                "--",
                "killall5",
                "-9",
            ])
            .output();

        let ps_output = Command::new("ps").args(&["-ef"]).output()?;
        let ps_str = String::from_utf8_lossy(&ps_output.stdout);

        for line in ps_str.lines() {
            if line.contains(container_id) && (line.contains("chroot") || line.contains("unshare"))
            {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() > 1 {
                    process_to_kill = Some(parts[1].to_string());
                    break;
                }
            }
        }

        if let Some(pid) = &process_to_kill {
            println!("Killing process with PID {}", pid);

            let _ = Command::new("kill")
                .args(&["-9", "-", pid]) 
                .output()?;

            let _ = Command::new("kill").args(&["-9", pid]).output()?;
        }

        std::thread::sleep(std::time::Duration::from_millis(500));

        let _ = Command::new("ip")
            .args(&["netns", "delete", container_id])
            .output();
    }

    let _ = Command::new("iptables").args(&["-F", "FORWARD"]).output();

    Ok(())
}
