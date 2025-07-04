use crate::actions::types::{AuthToken, ImageManifest, ManifestResponse};
use reqwest::Client;
use std::fs;
use tokio::io::AsyncWriteExt;

pub async fn pull_image(image_tag: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("🔄 Pulling image: {}", image_tag);

    let (repository, tag) = parse_image_tag(image_tag);

    let client = Client::new();

    let token = get_auth_token(&client, &repository).await?;

    let manifest_response = get_manifest(&client, &repository, &tag, &token).await?;

    let image_manifest = match manifest_response {
        ManifestResponse::V2(manifest) => {
            println!("Image manifest schema version: {}", manifest.schema_version);
            manifest
        }
        ManifestResponse::List(manifest_list) => {
            println!("📋 Found manifest list, selecting platform...");

            let selected_manifest = manifest_list
                .manifests
                .iter()
                .find(|m| {
                    if let Some(platform) = &m.platform {
                        platform.os == "linux" && platform.architecture == "amd64"
                    } else {
                        false
                    }
                })
                .or_else(|| manifest_list.manifests.first())
                .ok_or("No suitable manifest found in manifest list")?;

            println!(
                "📋 Selected platform: {}/{}",
                selected_manifest
                    .platform
                    .as_ref()
                    .map(|p| p.os.as_str())
                    .unwrap_or("unknown"),
                selected_manifest
                    .platform
                    .as_ref()
                    .map(|p| p.architecture.as_str())
                    .unwrap_or("unknown")
            );

            get_manifest_by_digest(&client, &repository, &selected_manifest.digest, &token).await?
        }
    };

    let image_dir = format!("./images/{}/{}", repository.replace('/', "_"), tag);
    fs::create_dir_all(&image_dir)?;

    println!("📥 Downloading config...");
    download_blob(
        &client,
        &repository,
        &image_manifest.config.digest,
        &token,
        &image_dir,
    )
    .await?;

    for (i, layer) in image_manifest.layers.iter().enumerate() {
        println!(
            "📥 Downloading layer {}/{} ({})",
            i + 1,
            image_manifest.layers.len(),
            format_size(layer.size)
        );
        download_blob(&client, &repository, &layer.digest, &token, &image_dir).await?;
    }

    let manifest_path = format!("{}/manifest.json", image_dir);
    let manifest_json = serde_json::to_string_pretty(&image_manifest)?;
    fs::write(manifest_path, manifest_json)?;

    println!("✅ Successfully pulled {}", image_tag);
    Ok(())
}

pub fn parse_image_tag(image_tag: &str) -> (String, String) {
    if let Some(pos) = image_tag.rfind(':') {
        let repository = image_tag[..pos].to_string();
        let tag = image_tag[pos + 1..].to_string();

        let full_repository = if repository.contains('/') {
            repository
        } else {
            format!("library/{}", repository)
        };

        return (full_repository, tag);
    }

    let full_repository = if image_tag.contains('/') {
        image_tag.to_string()
    } else {
        format!("library/{}", image_tag)
    };

    (full_repository, "latest".to_string())
}

async fn get_auth_token(
    client: &Client,
    repository: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let auth_url = format!(
        "https://auth.docker.io/token?service=registry.docker.io&scope=repository:{}:pull",
        repository
    );

    let response: AuthToken = client.get(&auth_url).send().await?.json().await?;

    Ok(response.token)
}

async fn get_manifest(
    client: &Client,
    repository: &str,
    tag: &str,
    token: &str,
) -> Result<ManifestResponse, Box<dyn std::error::Error>> {
    let manifest_url = format!(
        "https://registry-1.docker.io/v2/{}/manifests/{}",
        repository, tag
    );

    let response = client
        .get(&manifest_url)
        .header("Authorization", format!("Bearer {}", token))
        .header(
            "Accept",
            "application/vnd.docker.distribution.manifest.v2+json,application/vnd.docker.distribution.manifest.list.v2+json",
        )
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(format!("Failed to get manifest: {}", response.status()).into());
    }

    let manifest: ManifestResponse = response.json().await?;
    Ok(manifest)
}

async fn get_manifest_by_digest(
    client: &Client,
    repository: &str,
    digest: &str,
    token: &str,
) -> Result<ImageManifest, Box<dyn std::error::Error>> {
    let manifest_url = format!(
        "https://registry-1.docker.io/v2/{}/manifests/{}",
        repository, digest
    );

    let response = client
        .get(&manifest_url)
        .header("Authorization", format!("Bearer {}", token))
        .header(
            "Accept",
            "application/vnd.docker.distribution.manifest.v2+json",
        )
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(format!("Failed to get manifest by digest: {}", response.status()).into());
    }

    let manifest: ImageManifest = response.json().await?;
    Ok(manifest)
}

async fn download_blob(
    client: &Client,
    repository: &str,
    digest: &str,
    token: &str,
    image_dir: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let blob_url = format!(
        "https://registry-1.docker.io/v2/{}/blobs/{}",
        repository, digest
    );

    let response = client
        .get(&blob_url)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(format!("Failed to download blob {}: {}", digest, response.status()).into());
    }

    let filename = digest.replace("sha256:", "");
    let file_path = format!("{}/{}", image_dir, filename);

    let bytes = response.bytes().await?;
    let mut file = tokio::fs::File::create(&file_path).await?;
    file.write_all(&bytes).await?;

    Ok(())
}

fn format_size(size: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut size = size as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    format!("{:.1} {}", size, UNITS[unit_index])
}
