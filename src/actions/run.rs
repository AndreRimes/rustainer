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
    _container_id: &str,
    _ports: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    println!("🌐 Setting up container networking...");
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

    let mut cmd = Command::new("unshare");
    cmd.args(&[
        "--mount",
        "--uts",
        "--ipc",
        "--pid",
        "--fork",
        "--mount-proc",
        "sh",
        "-c",
        &format!("chroot {} {}", rootfs_path, command.join(" ")),
    ]);

    cmd.args(&command);

    for (key, value) in env_vars {
        cmd.env(key, value);
    }

    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());

    println!("Executing: chroot {} {:?}", rootfs_path, command);

    let status = cmd.status()?;
    if !status.success() {
        return Err(format!("Container exited with code: {:?}", status.code()).into());
    }

    Ok(())
}
