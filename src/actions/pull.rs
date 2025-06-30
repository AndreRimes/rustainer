// filepath: /home/andre/programing/projetos/rustainer/src/actions/pull.rs
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tokio::io::AsyncWriteExt;

#[derive(Debug, Deserialize)]
struct ManifestResponse {
    #[serde(rename = "schemaVersion")]
    schema_version: i32,
    #[serde(rename = "mediaType")]
    media_type: String,
    config: Layer,
    layers: Vec<Layer>,
}

#[derive(Debug, Deserialize)]
struct Layer {
    #[serde(rename = "mediaType")]
    media_type: String,
    size: u64,
    digest: String,
}

#[derive(Debug, Deserialize)]
struct AuthToken {
    token: String,
}

pub async fn pull_image(image_tag: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("🔄 Pulling image: {}", image_tag);

    // Parse image tag (format: repository[:tag])
    let (repository, tag) = parse_image_tag(image_tag);

    // Create HTTP client
    let client = Client::new();

    // Get authentication token
    let token = get_auth_token(&client, &repository).await?;

    // Get image manifest
    let manifest = get_manifest(&client, &repository, &tag, &token).await?;

    // Create directory for storing image layers
    let image_dir = format!("./images/{}", repository.replace('/', "_"));
    fs::create_dir_all(&image_dir)?;

    // Download config blob
    println!("📥 Downloading config...");
    download_blob(
        &client,
        &repository,
        &manifest.config.digest,
        &token,
        &image_dir,
    )
    .await?;

    // Download each layer
    for (i, layer) in manifest.layers.iter().enumerate() {
        println!(
            "📥 Downloading layer {}/{} ({})",
            i + 1,
            manifest.layers.len(),
            format_size(layer.size)
        );
        download_blob(&client, &repository, &layer.digest, &token, &image_dir).await?;
    }

    // Save manifest
    let manifest_path = format!("{}/manifest.json", image_dir);
    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    fs::write(manifest_path, manifest_json)?;

    println!("✅ Successfully pulled {}", image_tag);
    Ok(())
}

fn parse_image_tag(image_tag: &str) -> (String, String) {
    if let Some(pos) = image_tag.rfind(':') {
        let repository = image_tag[..pos].to_string();
        let tag = image_tag[pos + 1..].to_string();

        // Handle official images (no namespace)
        let full_repository = if repository.contains('/') {
            repository
        } else {
            format!("library/{}", repository)
        };

        (full_repository, tag)
    } else {
        // Default to 'latest' tag if not specified
        let full_repository = if image_tag.contains('/') {
            image_tag.to_string()
        } else {
            format!("library/{}", image_tag)
        };
        (full_repository, "latest".to_string())
    }
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
            "application/vnd.docker.distribution.manifest.v2+json",
        )
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(format!("Failed to get manifest: {}", response.status()).into());
    }

    let manifest: ManifestResponse = response.json().await?;
    Ok(manifest)
}

async fn download_blob(
    client: &Client,
    repository: &str,
    digest: &str,
    token: &str,
    output_dir: &str,
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

    // Create filename from digest (remove sha256: prefix)
    let filename = digest.replace("sha256:", "");
    let file_path = format!("{}/{}", output_dir, filename);

    // Write blob to file
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_image_tag() {
        assert_eq!(
            parse_image_tag("nginx"),
            ("library/nginx".to_string(), "latest".to_string())
        );
        assert_eq!(
            parse_image_tag("nginx:1.21"),
            ("library/nginx".to_string(), "1.21".to_string())
        );
        assert_eq!(
            parse_image_tag("ubuntu/nginx:latest"),
            ("ubuntu/nginx".to_string(), "latest".to_string())
        );
    }

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(512), "512.0 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1048576), "1.0 MB");
    }
}
