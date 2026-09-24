use super::Deserialize;
use super::Serialize;

// ---------------------------------------------------------------------------
// Types — releases.json
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageInfo {
    pub filename: String,
    pub extension: String,
    /// Size of the *download* — the compressed `.img.xz`. The card has to hold
    /// the decompressed image, which is several times larger, so this must
    /// never be compared against a card's capacity.
    pub size: u64,
    /// Size of the decompressed image, when the release publishes it. Absent
    /// today, so the UI falls back to [`MIN_CARD_BYTES`].
    #[serde(default)]
    pub uncompressed_size: Option<u64>,
    pub download_url: String,
    pub checksum_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub prerelease: bool,
    pub created_at: String,
    pub image: ImageInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleasesResponse {
    pub releases: Vec<Release>,
    /// Stamped from [`MIN_CARD_BYTES`] on the way out, so the card-size warning
    /// has one source of truth instead of a constant copied into the frontend.
    #[serde(default)]
    pub min_card_bytes: u64,
}

// ---------------------------------------------------------------------------
// Types — block devices & progress
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockDevice {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub size_human: String,
    pub removable: bool,
    pub mounted: bool,
    pub mount_points: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub percent: f64,
    pub speed_bps: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlashProgress {
    pub phase: FlashPhase,
    pub bytes_processed: u64,
    pub total_bytes: u64,
    pub percent: f64,
    /// Rate of the phase in flight. The write is the longest wait in the app,
    /// so a bar alone answers "is it moving" but not "how long".
    pub speed_bps: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FlashPhase {
    Decompressing,
    Writing,
    Verifying,
    Customizing,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadResult {
    pub image_path: String,
    pub checksum: String,
    pub cached: bool,
}
