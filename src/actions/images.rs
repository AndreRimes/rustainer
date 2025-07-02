use crate::actions::types::ImageManifest;
use std::{fs, path::Path, time::SystemTime};

struct ImageInfo {
    repository: String,
    tag: String,
    image_id: String,
    created: String,
    size: u64,
    // layers: usize,
}

pub async fn list_images() -> Result<(), Box<dyn std::error::Error>> {
    let images_dir = "./images";

    if !Path::new(images_dir).exists() {
        println!("No images found. Use 'rustainer pull <image>' to download images.");
        return Ok(());
    }

    let mut images = Vec::new();

    for entry in fs::read_dir(images_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            if let Some(image_info) = parse_image_directory(&path).await? {
                images.push(image_info);
            }
        }
    }

    if images.is_empty() {
        println!("No images found.");
        return Ok(());
    }

    images.sort_by(|a, b| a.repository.cmp(&b.repository));

    print_images_table(&images);

    Ok(())
}

async fn parse_image_directory(
    path: &Path,
) -> Result<Option<ImageInfo>, Box<dyn std::error::Error>> {
    let manifest_path = path.join("manifest.json");

    if !manifest_path.exists() {
        return Ok(None);
    }

    let manifest_content = fs::read_to_string(&manifest_path)?;
    let manifest: ImageManifest = serde_json::from_str(&manifest_content)?;

    let dir_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let repository = dir_name.replace('_', "/");

    let metadata = fs::metadata(&manifest_path)?;
    let created = metadata
        .created()
        .or_else(|_| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);

    let created_str = format_time(created);

    let mut total_size = manifest.config.size;
    for layer in &manifest.layers {
        total_size += layer.size;
    }

    let image_id = manifest
        .config
        .digest
        .strip_prefix("sha256:")
        .unwrap_or(&manifest.config.digest)
        .chars()
        .take(12)
        .collect();

    let img_tag = if let Some(tag) = manifest.tag {
        tag
    } else {
        "latest".to_owned()
    };

    Ok(Some(ImageInfo {
        repository,
        tag: img_tag,
        image_id,
        created: created_str,
        size: total_size,
        // layers: manifest.layers.len(),
    }))
}

fn format_size(size: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut size = size as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    format!("{:.1}{}", size, UNITS[unit_index])
}

fn format_time(time: SystemTime) -> String {
    let elapsed = time.elapsed().unwrap_or_default();
    let secs = elapsed.as_secs();

    if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

fn print_images_table(images: &[ImageInfo]) {
    println!(
        "{:<30} {:<10} {:<15} {:<15} {:<10}",
        "REPOSITORY", "TAG", "IMAGE ID", "CREATED", "SIZE"
    );

    for image in images {
        println!(
            "{:<30} {:<10} {:<15} {:<15} {:<10}",
            image.repository,
            image.tag,
            image.image_id,
            image.created,
            format_size(image.size)
        );
    }
}
