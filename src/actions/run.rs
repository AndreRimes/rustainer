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

    create_network_namespace(container_id)?;

    ensure_bridge_exists("rustainer0")?;

    let (host_veth, container_veth) = create_veth_pair(container_id)?;

    attach_veth_to_bridge(&host_veth, "rustainer0")?;
    attach_veth_to_bridge(&container_veth, "rustainer0")?;

    move_veth_to_namespace(&container_veth, container_id)?;

    configure_container_interface(container_id, &container_veth)?;

    if !ports.is_empty() {
        setup_port_forwarding(container_id, ports)?;
    }

    println!("✅ Container network configured successfully");
    Ok(())
}

fn create_network_namespace(container_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Creating network namespace for container: {}", container_id);

    let output = Command::new("ip")
        .args(&["netns", "add", container_id]) // ip netns add <container_id>
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.contains("File exists") {
            return Err(format!("Failed to create network namespace: {}", stderr).into());
        }
    }

    Ok(())
}

fn ensure_bridge_exists(bridge_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("🌉 Ensuring bridge exists: {}", bridge_name);

    let check_output = Command::new("ip")
        .args(&["link", "show", bridge_name])
        .output()?;

    if !check_output.status.success() {
        println!("🔧 Creating bridge: {}", bridge_name);

        let output = Command::new("ip")
            .args(&["link", "add", "name", bridge_name, "type", "bridge"])
            .output()?;

        if !output.status.success() {
            return Err(format!(
                "Failed to create bridge: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }

        let output = Command::new("ip")
            .args(&["addr", "add", "172.18.0.1/16", "dev", bridge_name])
            .output()?;

        if !output.status.success() {
            return Err(format!(
                "Failed to assign IP to bridge: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }

        let output = Command::new("ip")
            .args(&["link", "set", bridge_name, "up"])
            .output()?;

        if !output.status.success() {
            return Err(format!(
                "Failed to bring up bridge: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }

        setup_nat_rules(bridge_name)?;

        println!("✅ Bridge {} created and activated", bridge_name);
    } else {
        println!("✅ Bridge {} already exists", bridge_name);
    }

    Ok(())
}

fn setup_nat_rules(bridge_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("🔧 Setting up NAT rules for bridge: {}", bridge_name);

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

    let output = Command::new("iptables")
        .args(&[
            "-t",
            "nat",
            "-A",
            "POSTROUTING",
            "-s",
            "172.18.0.0/16",
            "!",
            "-o",
            bridge_name,
            "-j",
            "MASQUERADE",
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.contains("iptables: Chain already exists") {
            println!("⚠️ Warning: Could not add NAT rule: {}", stderr);
        }
    }

    let output = Command::new("iptables")
        .args(&[
            "-A",
            "FORWARD",
            "-i",
            bridge_name,
            "-o",
            bridge_name,
            "-j",
            "ACCEPT",
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.contains("iptables: Chain already exists") {
            println!("⚠️ Warning: Could not add FORWARD rule: {}", stderr);
        }
    }

    Ok(())
}

fn parse_port_mapping(port_mapping: &str) -> Result<(u16, u16), Box<dyn std::error::Error>> {
    let parts: Vec<&str> = port_mapping.split(':').collect();

    if parts.len() != 2 {
        return Err("Port mapping format should be 'port' or 'host_port:container_port'".into());
    }

    let host_port = parts[0].parse::<u16>()?;
    let container_port = parts[1].parse::<u16>()?;
    Ok((host_port, container_port))
}

fn create_veth_pair(container_id: &str) -> Result<(String, String), Box<dyn std::error::Error>> {
    let host_veth = format!("veth-{}", &container_id[..8]);
    let container_veth = format!("cveth-{}", &container_id[..8]);

    println!(
        "🔗 Creating veth pair: {} <-> {}",
        host_veth, container_veth
    );

    let output = Command::new("ip")
        .args(&[
            "link",
            "add",
            &host_veth,
            "type",
            "veth",
            "peer",
            "name",
            &container_veth,
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
        .args(&["link", "set", &host_veth, "up"])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "Failed to bring up host veth: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    Ok((host_veth, container_veth))
}

fn attach_veth_to_bridge(
    veth_name: &str,
    bridge_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("🔌 Attaching {} to bridge {}", veth_name, bridge_name);

    let output = Command::new("ip")
        .args(&["link", "set", veth_name, "master", bridge_name])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "Failed to attach veth to bridge: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    Ok(())
}

fn move_veth_to_namespace(
    veth_name: &str,
    namespace: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("📦 Moving {} to namespace {}", veth_name, namespace);

    let output = Command::new("ip")
        .args(&["link", "set", veth_name, "netns", namespace])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "Failed to move veth to namespace: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    Ok(())
}

fn configure_container_interface(
    container_id: &str,
    interface_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("⚙️ Configuring container interface: {}", interface_name);

    let ip_suffix = (container_id.len() % 254) + 2;
    let container_ip = format!("172.18.0.{}/16", ip_suffix);

    // 1. Renomear interface para eth0
    let output = Command::new("ip")
        .args(&[
            "netns",
            "exec",
            container_id,
            "ip",
            "link",
            "set",
            interface_name,
            "name",
            "eth0",
        ])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "Failed to rename interface: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    // 2. Ativar loopback primeiro
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

    // 3. Configurar IP da interface
    let output = Command::new("ip")
        .args(&[
            "netns",
            "exec",
            container_id,
            "ip",
            "addr",
            "add",
            &container_ip,
            "dev",
            "eth0",
        ])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "Failed to assign IP to container interface: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    // 4. Ativar interface eth0
    let output = Command::new("ip")
        .args(&[
            "netns",
            "exec",
            container_id,
            "ip",
            "link",
            "set",
            "eth0",
            "up",
        ])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "Failed to bring up container interface: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }

    // 5. Aguardar um pouco para a interface ficar pronta
    std::thread::sleep(std::time::Duration::from_millis(100));

    // 6. Configurar rota padrão
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
            "172.18.0.1",
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.contains("File exists") {
            return Err(format!("Failed to add default route: {}", stderr).into());
        }
    }

    // 7. Debug: verificar configuração final
    println!("📡 Container IP configured: {}", container_ip);

    // Verificar se tudo está funcionando
    let output = Command::new("ip")
        .args(&["netns", "exec", container_id, "ip", "addr", "show", "eth0"])
        .output()?;

    if output.status.success() {
        println!(
            "🔍 Interface eth0 status: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }

    Ok(())
}

fn setup_port_forwarding(
    container_id: &str,
    ports: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "🔀 Setting up port forwarding for container: {}",
        container_id
    );

    let ip_suffix = (container_id.len() % 254) + 2;
    let container_ip = format!("172.18.0.{}", ip_suffix);

    for port_mapping in ports {
        let (host_port, container_port) = parse_port_mapping(port_mapping)?;

        println!(
            "🔁 Forwarding host port {} -> container port {} ({})",
            host_port, container_port, container_ip
        );

        let output = Command::new("iptables")
            .args(&[
                "-t",
                "nat",
                "-A",
                "PREROUTING",
                "-p",
                "tcp",
                "--dport",
                &host_port.to_string(),
                "-j",
                "DNAT",
                "--to-destination",
                &format!("{}:{}", container_ip, container_port),
            ])
            .output()?;

        if !output.status.success() {
            return Err(format!(
                "Failed to set up DNAT rule: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }

        let output = Command::new("iptables")
            .args(&[
                "-A",
                "FORWARD",
                "-p",
                "tcp",
                "-d",
                &container_ip,
                "--dport",
                &container_port.to_string(),
                "-j",
                "ACCEPT",
            ])
            .output()?;

        if !output.status.success() {
            return Err(format!(
                "Failed to set up FORWARD rule: {}",
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

    // Set environment variables
    for (key, value) in env_vars {
        cmd.env(key, value);
    }

    // Configure stdio
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
