use std::{
    collections::HashMap,
    fs,
    path::Path,
    process::{Command, Stdio},
};

use crate::actions::{self, types::ImageManifest};

#[derive(Debug)]
pub struct RunConfig {
    pub image: String,
    pub name: Option<String>,
    pub detach: bool,
    pub interactive: bool,
    pub tty: bool,
    pub env_vars: Vec<String>,
    pub volumes: Vec<String>,
    pub ports: Vec<String>,
    pub command: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize)]
struct ImageConfig {
    #[serde(rename = "Env", default)]
    env: Vec<String>,
    #[serde(rename = "Cmd", default)]
    cmd: Vec<String>,
    #[serde(rename = "Entrypoint", default)]
    entrypoint: Vec<String>,
    #[serde(rename = "WorkingDir", default)]
    working_dir: String,
    #[serde(rename = "User", default)]
    user: String,
}

pub async fn run_container(config: RunConfig) -> Result<(), Box<dyn std::error::Error>> {
    let (repository, tag) = actions::pull::parse_image_tag(&config.image);
    let image_path = find_local_image(&repository, &tag)?;

    let manifest = load_image_manifest(&image_path)?;
    let image_config = load_image_config(&image_path, &manifest.config.digest)?;

    use std::time::{SystemTime, UNIX_EPOCH};
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let container_id = format!("rustainer_{}", timestamp);
    let container_path = create_container_filesystem(&container_id, &image_path, &manifest).await?;

    setup_container_networking(&container_id, &config.ports)?;

    let env_vars = prepare_environment(&config.env_vars, &image_config.env);
    let cmd = prepare_command(&config.command, &image_config.cmd, &image_config.entrypoint);

    execute_container(&container_id, &container_path, cmd, env_vars, &config).await?;

    Ok(())
}

fn find_local_image(repository: &str, tag: &str) -> Result<String, Box<dyn std::error::Error>> {
    let image_path = format!("./images/{}/{}", repository.replace('/', "_"), tag);
    if !Path::new(&image_path).exists() {
        return Err(format!(
            "Image {}:{} not found locally. You need to pull it first.",
            repository, tag
        )
        .into());
    }

    Ok(image_path)
}

fn load_image_manifest(image_path: &str) -> Result<ImageManifest, Box<dyn std::error::Error>> {
    let manifest_path = format!("{}/manifest.json", image_path);
    let manifest_content = fs::read_to_string(manifest_path)?;
    let manifest: ImageManifest = serde_json::from_str(&manifest_content)?;
    Ok(manifest)
}

fn load_image_config(
    image_path: &str,
    config_digest: &str,
) -> Result<ImageConfig, Box<dyn std::error::Error>> {
    let config_filename = config_digest.replace("sha256:", "");
    let config_path = format!("{}/{}", image_path, config_filename);

    let config_content = fs::read_to_string(config_path)?;
    let config: ImageConfig = serde_json::from_str(&config_content)?;
    Ok(config)
}

async fn create_container_filesystem(
    container_id: &str,
    image_path: &str,
    manifest: &ImageManifest,
) -> Result<String, Box<dyn std::error::Error>> {
    let container_path = format!("./containers/{}", container_id);
    let rootfs_path = format!("{}/rootfs", container_path);

    fs::create_dir_all(&rootfs_path)?;

    println!("Creating container filesystem");

    for (i, layer) in manifest.layers.iter().enumerate() {
        println!(
            "Etracting layer {}/{}: {}",
            i + 1,
            manifest.layers.len(),
            layer.digest
        );

        extract_layer(image_path, &layer.digest, &rootfs_path).await?;
    }

    Ok(container_path)
}

async fn extract_layer(
    image_path: &str,
    layer_digest: &str,
    rootfs_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let layer_filename = layer_digest.replace("sha256:", "");
    let layer_path = format!("{}/{}", image_path, layer_filename);

    let output = Command::new("tar")
        .args(["-xzf", &layer_path, "-C", rootfs_path])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "Failed to extract layer: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    Ok(())
}

fn setup_container_networking(
    container_id: &str,
    ports: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    println!("🌐 Setting up container networking...");

    let output = Command::new("sysctl")
        .args(&["-w", "net.ipv4.ip_forward=1"])
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "Failed to enable IP forwarding: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    create_container_namespace(container_id)?;

    create_host_switch("rustainer0")?;

    let (veth_container, _) = create_bridge(container_id)?;

    let container_ip = add_ip_to_network(container_id, &veth_container)?;

    add_routing_rules(container_id)?;

    setup_port_mapping(&container_ip, ports)?;

    Ok(())
}

fn create_container_namespace(container_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("ip")
        .args(&["netns", "add", container_id])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "Failed to create network namespace: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    Ok(())
}

fn create_host_switch(host_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let host_name = host_name.trim();

    let check_output = Command::new("ip")
        .args(&["link", "show", host_name])
        .output()?;

    if check_output.status.success() {
        return Ok(());
    }

    let output = Command::new("ip")
        .args(&["link", "add", host_name, "type", "bridge"])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "Failed to create host switch: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    let output = Command::new("ip")
        .args(&["link", "set", "dev", host_name, "up"])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "Failed to bring up host switch: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    Ok(())
}

fn create_bridge(container_id: &str) -> Result<(String, String), Box<dyn std::error::Error>> {
    let short_id: String = container_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect();

    let container_veth = format!("veth{}c", short_id);
    let host_veth = format!("veth{}h", short_id);

    let output = Command::new("ip")
        .args(&[
            "link",
            "add",
            &container_veth,
            "type",
            "veth",
            "peer",
            "name",
            &host_veth,
        ])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "Failed to create veth pair: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    let output = Command::new("ip")
        .args(&["link", "set", &container_veth, "netns", container_id])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "Failed to move veth to container namespace: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    let output = Command::new("ip")
        .args(&["link", "set", &host_veth, "master", "rustainer0"])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "Failed to attach host veth to bridge: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    let output = Command::new("ip")
        .args(&["link", "set", &host_veth, "up"])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "Failed to bring up host veth: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    println!(
        "🔗 Created veth pair: {} (container) <-> {} (host)",
        container_veth, host_veth
    );

    Ok((container_veth, host_veth))
}

fn add_ip_to_network(
    container_id: &str,
    veth_container: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new("ip")
        .args(&["addr", "add", "172.19.0.1/16", "dev", "rustainer0"])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "Failed to add IP to host: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    let ip_suffix = (container_id.len() % 254) + 2;
    let container_ip = format!("172.19.0.{}", ip_suffix);

    let output = Command::new("ip")
        .args(&[
            "netns",
            "exec",
            container_id,
            "ip",
            "addr",
            "add",
            &format!("{}/16", container_ip),
            "dev",
            veth_container,
        ])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "Failed to add IP to container: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    let output = Command::new("ip")
        .args(&[
            "netns",
            "exec",
            container_id,
            "ip",
            "link",
            "set",
            veth_container,
            "up",
        ])
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "Failed to bring up veth in container namespace: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    let output = Command::new("ip")
        .args(&[
            "netns",
            "exec",
            container_id,
            "ip",
            "route",
            "add",
            "default",
            "via",
            "172.19.0.1",
        ])
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "Failed to add default route: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    println!("🖥️ Container {} IP: {}", container_id, container_ip);

    Ok(container_ip)
}

fn add_routing_rules(container_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("🌐 Adding routing rules for container: {}", container_id);

    let output = Command::new("ip")
        .args(&[
            "netns",
            "exec",
            container_id,
            "ip",
            "link",
            "set",
            "lo",
            "up",
        ])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "Failed to bring up loopback: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    let output = Command::new("iptables")
        .args(&[
            "-t",
            "nat",
            "-A",
            "POSTROUTING",
            "-s",
            "172.19.0.0/16",
            "!",
            "-o",
            "rustainer0",
            "-j",
            "MASQUERADE",
        ])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "Failed to set up NAT rules: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    Ok(())
}

fn setup_port_mapping(
    container_ip: &str,
    ports: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    for port_mapping in ports {
        let parts: Vec<&str> = port_mapping.split(':').collect();
        if parts.len() != 2 {
            return Err(format!(
                "Invalid port mapping format: {}. Expected format is <host_port>:<container_port>",
                port_mapping
            )
            .into());
        }
        let host_port = parts[0];
        let container_port = parts[1];

        let output = Command::new("iptables")
            .args(&[
                "-t",
                "nat",
                "-A",
                "PREROUTING",
                "-p",
                "tcp",
                "--dport",
                host_port,
                "-j",
                "DNAT",
                "--to-destination",
                &format!("{}:{}", container_ip, container_port),
            ])
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "Error configuring DNAT (PREROUTING) for port {}: {}",
                host_port,
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }

        let output = Command::new("iptables")
            .args(&[
                "-t",
                "nat",
                "-A",
                "OUTPUT",
                "-p",
                "tcp",
                "--dport",
                host_port,
                "-j",
                "DNAT",
                "--to-destination",
                &format!("{}:{}", container_ip, container_port),
            ])
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "Error configuring DNAT (OUTPUT) for port {}: {}",
                host_port,
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }

        let output = Command::new("iptables")
            .args(&[
                "-A",
                "FORWARD",
                "-i",
                "rustainer0",
                "!",
                "-o",
                "rustainer0",
                "-j",
                "ACCEPT",
            ])
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "Error allowing FORWARD for host to container: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }

        let output = Command::new("iptables")
            .args(&[
                "-A",
                "FORWARD",
                "-o",
                "rustainer0",
                "-m",
                "conntrack",
                "--ctstate",
                "RELATED,ESTABLISHED",
                "-j",
                "ACCEPT",
            ])
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "Erro ao permitir FORWARD para conexões estabelecidas: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }

        let output = Command::new("iptables")
            .args(&[
                "-A",
                "FORWARD",
                "-d",
                container_ip,
                "-p",
                "tcp",
                "--dport",
                container_port,
                "-o",
                "rustainer0",
                "-j",
                "ACCEPT",
            ])
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "Erro ao permitir FORWARD para o container: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }

        let output = Command::new("iptables")
            .args(&[
                "-A",
                "FORWARD",
                "-o",
                "rustainer0",
                "-m",
                "conntrack",
                "--ctstate",
                "RELATED,ESTABLISHED",
                "-j",
                "ACCEPT",
            ])
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "Erro ao permitir FORWARD para conexões estabelecidas (container): {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
    }

    Ok(())
}

fn prepare_environment(user_envs: &[String], image_envs: &[String]) -> HashMap<String, String> {
    let mut env_map = HashMap::new();

    // USER=root -> USER : root
    for env_var in image_envs {
        if let Some(pos) = env_var.find('=') {
            let key = env_var[..pos].to_string();
            let value = env_var[pos + 1..].to_string();
            env_map.insert(key, value);
        }
    }

    for env_var in user_envs {
        if let Some(pos) = env_var.find('=') {
            let key = env_var[..pos].to_string();
            let value = env_var[pos + 1..].to_string();
            env_map.insert(key, value);
        }
    }

    env_map
}

fn prepare_command(
    user_cmd: &Option<Vec<String>>,
    image_cmd: &[String],
    image_entrypoint: &[String],
) -> Vec<String> {
    match user_cmd {
        Some(cmd) => cmd.clone(),
        None => {
            if !image_entrypoint.is_empty() {
                let mut full_cmd = image_entrypoint.to_vec();
                full_cmd.extend(image_cmd.iter().cloned());
                full_cmd
            } else if !image_cmd.is_empty() {
                image_cmd.to_vec()
            } else {
                vec!["/bin/sh".to_string()]
            }
        }
    }
}

async fn execute_container(
    container_id: &str,
    container_path: &str,
    command: Vec<String>,
    env_vars: HashMap<String, String>,
    config: &RunConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    let rootfs_path = format!("{}/rootfs", container_path);

    if command.is_empty() {
        return Err("No command specified to run in the container".into());
    }

    let mut cmd = Command::new("ip");
    cmd.args(&[
        "netns",
        "exec",
        container_id,
        "unshare",
        "--mount",
        "--uts",
        "--ipc",
        "--pid",
        "--fork",
        "--mount-proc",
        "chroot",
        &rootfs_path,
    ]);
    cmd.args(&command);

    for (key, value) in env_vars {
        cmd.env(key, value);
    }

    if config.detach {
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());
    } else {
        cmd.stdin(Stdio::inherit());
        cmd.stdout(Stdio::inherit());
        cmd.stderr(Stdio::inherit());
    }

    println!("🏃 Executing in network namespace: ip netns exec {} unshare --mount --uts --ipc --pid --fork --mount-proc chroot {} {:?}", 
             container_id, rootfs_path, command);

    if config.detach {
        // Executar em background
        let child = cmd.spawn()?;
        println!(
            "🔧 Container running in background with PID: {}",
            child.id()
        );

        // Aguardar um pouco para dar tempo do nginx inicializar
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        println!("✅ Container started successfully");
        // Não limpar recursos imediatamente quando em detach mode
        return Ok(());
    } else {
        // Executar em foreground
        let status = cmd.status()?;

        // Limpar recursos de rede após execução
        if let Err(e) = cleanup_container_networking(container_id) {
            println!("⚠️ Warning: Failed to cleanup networking: {}", e);
        }

        if !status.success() {
            return Err(format!("Container exited with code: {:?}", status.code()).into());
        }
    }

    Ok(())
}

pub fn cleanup_container_networking(container_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("🧹 Cleaning up networking for container: {}", container_id);

    let output = Command::new("ip")
        .args(&["netns", "delete", container_id])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.contains("No such file or directory") {
            println!("⚠️ Warning: Could not delete network namespace: {}", stderr);
        }
    }

    Ok(())
}
