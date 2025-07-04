use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ManifestResponse {
    V2(ImageManifest),
    List(ManifestList),
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ImageManifest {
    #[serde(rename = "schemaVersion")]
    pub schema_version: i32,
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub config: Layer,
    pub layers: Vec<Layer>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ManifestList {
    #[serde(rename = "schemaVersion")]
    pub schema_version: i32,
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub manifests: Vec<PlatformManifest>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct PlatformManifest {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub size: u64,
    pub digest: String,
    pub platform: Option<Platform>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Platform {
    pub architecture: String,
    pub os: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Layer {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub size: u64,
    pub digest: String,
}

#[derive(Debug, Deserialize)]
pub struct AuthToken {
    pub token: String,
}
