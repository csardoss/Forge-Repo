use std::path::Path;

use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

/// Download a file from a presigned URL, verifying SHA-256 if expected.
/// Returns the actual SHA-256 hex digest.
pub async fn download_to_file(
    url: &str,
    dest: &Path,
    expected_sha256: Option<&str>,
    expected_size: Option<i64>,
) -> Result<String> {
    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .send()
        .await
        .context("Download request failed")?;

    if !resp.status().is_success() {
        bail!("Download failed with status {}", resp.status());
    }

    let total = resp
        .content_length()
        .or(expected_size.map(|s| s as u64));

    let pb = match total {
        Some(size) => {
            let pb = ProgressBar::new(size);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("  [{bar:30.cyan/blue}] {bytes}/{total_bytes} {bytes_per_sec}")
                    .unwrap()
                    .progress_chars("=> "),
            );
            pb
        }
        None => {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::default_spinner()
                    .template("  {spinner} {bytes} downloaded")
                    .unwrap(),
            );
            pb
        }
    };

    let mut file = File::create(dest)
        .await
        .with_context(|| format!("Failed to create {}", dest.display()))?;
    let mut hasher = Sha256::new();
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("Error reading download stream")?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        pb.inc(chunk.len() as u64);
    }

    file.flush().await?;
    pb.finish_and_clear();

    let actual_sha256 = format!("{:x}", hasher.finalize());

    if let Some(expected) = expected_sha256 {
        if actual_sha256 != expected {
            bail!(
                "SHA-256 mismatch: expected {expected}, got {actual_sha256}"
            );
        }
    }

    Ok(actual_sha256)
}
