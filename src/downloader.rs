use anyhow::{Context, Result, anyhow};
use config::{MAX_CONCURRENT_DOWNLOADS, PixivResponse};
use futures::stream::{self, StreamExt};
use helpers::{
    filter_valid_urls, format_filename, generate_image_urls, normalize_image_url, set_headers,
};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::{
    Client,
    header::{CONTENT_LENGTH, RANGE},
};
use scraper::{Html, Selector};
use std::{
    fs::{OpenOptions, create_dir_all, rename},
    io::Write,
    path::{Path, PathBuf},
};
use url::Url;

use crate::{config, helpers};

pub async fn download_pixiv_artwork(artwork_url: &str) -> Result<()> {
    let client = Client::new();
    let headers = set_headers();

    let response = client
        .get(artwork_url)
        .headers(headers)
        .send()
        .await?
        .text()
        .await?;
    let document = Html::parse_document(&response);
    let selector = Selector::parse("meta#meta-preload-data")
        .map_err(|e| anyhow!("Selector parse error: {}", e))?;
    let meta_tag = document.select(&selector).next();
    if meta_tag.is_none() {
        eprintln!("Could not find `meta-preload-data`, request may be restricted");
    }
    let meta_content = meta_tag
        .and_then(|tag| tag.value().attr("content"))
        .context("`content` attribute not found in `meta` tag")?;
    let pixiv_response: PixivResponse = serde_json::from_str(meta_content)?;
    let illust_data = pixiv_response
        .illust
        .values()
        .next()
        .context("No illustration data")?;
    let artwork_id = Url::parse(artwork_url)?
        .path_segments()
        .and_then(|mut segement| segement.next_back())
        .context("Invalid URL format")?
        .to_string();

    let pixiv_dir = dirs::picture_dir().unwrap_or_else(|| PathBuf::from("~/Pictures/"));
    let artwork_dir = pixiv_dir.join("pixiv").join(&artwork_id);
    let _ = create_dir_all(&artwork_dir);

    let page_count = illust_data.page_count as usize;
    let image_urls: Vec<String> = match &illust_data.urls.original {
        Some(original_url) if original_url.contains(&artwork_id) => (0..page_count)
            .map(|seq| original_url.replace("p0", &format!("p{}", seq)))
            .collect(),
        _ => {
            let urls = illust_data
                .user_illusts
                .as_ref()
                .map(|user_illusts| {
                    user_illusts
                        .values()
                        .filter_map(|illust| illust.as_ref()?.url.as_ref()) // handle `Option<UserIllust>`
                        .filter(|base_url| base_url.contains(&artwork_id)) // filter non-target URL
                        .map(|base_url| normalize_image_url(base_url)) // convert to original URL
                        .flat_map(|normalized_url| generate_image_urls(&normalized_url, page_count))
                        .collect::<Vec<String>>()
                })
                .unwrap_or_default();

            filter_valid_urls(urls).await
        }
    };
    if image_urls.is_empty() {
        eprintln!("No iamge URL found!");
    }

    let m = MultiProgress::new();
    let sty = ProgressStyle::with_template(
        "{spinner:.green}[{eta}][{bar:40.cyan/blue}] {bytes}/{total_bytes} {msg}",
    )
    .unwrap()
    .progress_chars("#>-");

    stream::iter(image_urls.into_iter().map(|url| {
        let client = client.clone();
        let artwork_dir = artwork_dir.clone();
        let pb = m.add(ProgressBar::new(page_count as u64));
        pb.set_style(sty.clone());
        pb.set_message(url.split('/').next_back().unwrap_or(&url).to_string());
        async move {
            let _ = (download_image(client, &url, &artwork_dir, &pb).await).is_err();
            pb.inc(1);
        }
    }))
    .buffer_unordered(MAX_CONCURRENT_DOWNLOADS)
    .collect::<Vec<()>>()
    .await;

    // pb.finish_with_message("Download complete");
    Ok(())
}

async fn download_image(client: Client, url: &str, dir: &Path, pb: &ProgressBar) -> Result<()> {
    let filename = format_filename(url).context("Failed to format filename")?;
    let filepath = dir.join(&filename);
    let temp_path = filepath.with_extension("part");
    if filepath.exists() {
        pb.finish_with_message(format!("{filename} already exists"));
        return Ok(());
    }
    let existing_size = temp_path.metadata().map(|s| s.len()).unwrap_or(0);
    let mut headers = set_headers();
    headers.insert(RANGE, format!("bytes={}-", existing_size).parse()?);

    let response = client.get(url).headers(headers).send().await?;
    if !response.status().is_success() {
        pb.finish_with_message("Failed to download");
        return Err(anyhow!("HTTP error: {}", response.status()));
    }
    let total_size = if let Some(content_range) = response.headers().get("Content-Range") {
        content_range
            .to_str()?
            .split('/')
            .nth(1)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
    } else {
        response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
    };

    pb.set_length(total_size);
    pb.set_position(existing_size);

    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(&temp_path)?;

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk)?;
        pb.inc(chunk.len() as u64);
    }

    pb.finish_with_message(format!("{filename} 🗹"));
    rename(temp_path, filepath)?;
    Ok(())
}
