use anyhow::{Context, Result, anyhow};
use config::PixivResponse;
use futures::stream::{self, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use regex::Regex;
use reqwest::{
    Client,
    header::{self, ACCEPT, ACCEPT_LANGUAGE, CONNECTION, HeaderValue, RANGE, REFERER, USER_AGENT},
};
use scraper::{Html, Selector};
use std::{
    fs::{self, OpenOptions, create_dir_all},
    io::Write,
    path::PathBuf,
    usize,
};
use url::Url;

mod config;

const MAX_CONCURRENT_DOWNLOADS: usize = 5;

fn normalize_image_url(base_url: &str) -> String {
    println!("before normalize image url: {}", base_url);
    let re_custom_thumb = Regex::new(r"/c/250x250_80_a2/custom-thumb").unwrap();
    let re_img_master = Regex::new(r"/c/250x250_80_a2/img-master").unwrap();
    let re_square_custom = Regex::new(r"_(?:square|custom)1200\.jpg$").unwrap();

    let base_url = re_custom_thumb.replace(base_url, "/img-original");
    let base_url = re_img_master.replace(&base_url, "/img-original");
    let base_url = re_square_custom.replace(&base_url, ".jpg");

    base_url.into_owned()
}

fn format_filename(url: &str) -> Option<String> {
    let re = Regex::new(r"/(\d+)_p(\d+)(\.\w+)$").unwrap();
    if let Some(caps) = re.captures(url) {
        let artwork_id = &caps[1];
        let seq: u32 = caps[2].parse().ok()?;
        let extension = &caps[3];
        Some(format!("{}_p{:03}{}", artwork_id, seq, extension))
    } else {
        None
    }
}

async fn download_pixiv_artwork(artwork_url: &str) -> Result<()> {
    let client = Client::new();
    let mut headers = header::HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(
            "Mozilla/5.0 (X11; Linux x86_64; rv:137.0) Gecko/20100101 Firefox/137.0",
        ),
    );
    headers.insert(
        ACCEPT,
        HeaderValue::from_static(
            "image/avif,image/webp,image/png,image/svg+xml,image/*;q=0.8,*/*;q=0.5",
        ),
    );
    headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.5"));
    headers.insert(REFERER, HeaderValue::from_static("http://www.pixiv.net/"));
    headers.insert(CONNECTION, HeaderValue::from_static("keep-alive"));
    headers.insert("Sec-Fetch-Dest", HeaderValue::from_static("image"));
    headers.insert("Sec-Fetch-Mode", HeaderValue::from_static("no-cors"));
    headers.insert("Sec-Fetch-Site", HeaderValue::from_static("cross-site"));
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
    let meta_content = meta_tag.unwrap().value().attr("content").unwrap_or("");
    let pixiv_response: PixivResponse = serde_json::from_str(meta_content)?;
    let illust_data = pixiv_response
        .illust
        .values()
        .next()
        .context("No illustration data")?;
    let artwork_id = Url::parse(artwork_url)?
        .path_segments()
        .and_then(|segement| segement.last())
        .context("Invalid URL format")?
        .to_string();

    let pixiv_dir = dirs::picture_dir().unwrap_or_else(|| PathBuf::from("~/Pictures/"));
    let artwork_dir = pixiv_dir.join("pixiv").join(&artwork_id);
    let _ = create_dir_all(&artwork_dir);

    // "urls": Object {
    //     "original": Null,
    // },
    // "userIllusts": Object {
    //     "128183539": Object {
    //         "tags": Array [
    //             String("R-18"),
    //             String("ライダー(Fate/staynight)"),
    //             String("メドゥーサ(Fate)"),
    //         ],
    //         "title": String("【No.8】"),
    //         "titleCaptionTranslation": Object {
    //             "workCaption": Null,
    //             "workTitle": Null,
    //         },
    //         "updateDate": String("2025-03-14T02:58:10+09:00"),
    //         "url": String("https://i.pximg.net/c/250x250_80_a2/img-master/img/2025/03/14/02/58/10/128183539_p0_square1200.jpg"),
    let mut image_urls: Vec<String> = Vec::new();
    let page_count = illust_data.page_count as usize;
    match &illust_data.urls.original {
        Some(original_url) => {
            if original_url.contains(&artwork_id) {
                image_urls.extend(
                    (0..page_count).map(|seq| original_url.replace("p0", &format!("p{}", seq))),
                );
            }
        }
        None => {
            if let Some(user_illusts) = &illust_data.user_illusts {
                for (_, user_illust) in user_illusts {
                    if let Some(illust) = user_illust {
                        if let Some(base_url) = &illust.url {
                            if base_url.contains(&artwork_id) {
                                let normalized_url = normalize_image_url(base_url);
                                image_urls.extend(
                                    (0..page_count).map(|seq| {
                                        normalized_url.replace("p0", &format!("p{}", seq))
                                    }),
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    if image_urls.is_empty() {
        eprintln!("No iamge URL found!");
    }

    // let pb = ProgressBar::new(image_urls.len() as u64);
    // pb.set_style(
    //     ProgressStyle::default_bar()
    //         .template("{bar:40} {pos}/{len} {msg}")
    //         .unwrap(),
    // );

    // let m = MultiProgress::new();
    // let sty = ProgressStyle::with_template(
    //     "[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}",
    // )
    // .unwrap()
    // .progress_chars("##-");
    //
    // let pb = m.add(ProgressBar::new(image_urls.len() as u64));

    stream::iter(image_urls.into_iter().map(|url| {
        let client = client.clone();
        // let pb = pb.clone();
        let artwork_dir = artwork_dir.clone();
        async move {
            if let Err(e) = download_image(client, &url, &artwork_dir).await {
                eprintln!("Error downloading {}: {}", url, e);
            }
            // pb.inc(1);
        }
    }))
    .buffer_unordered(MAX_CONCURRENT_DOWNLOADS)
    .collect::<Vec<()>>()
    .await;

    // pb.finish_with_message("Download complete");
    Ok(())
}

async fn download_image(client: Client, url: &str, dir: &PathBuf) -> Result<()> {
    // let filename = url.split('/').last().context("Invalid URL")?;
    let filename = format_filename(url).context("Failed to format filename")?;
    let filepath = dir.join(&filename);
    let temp_path = filepath.with_extension("part");
    if filepath.exists() {
        // println!("[SKIP] {} already downloaded", filename);
        return Ok(());
    }
    let existing_size = temp_path.metadata().map(|s| s.len()).unwrap_or(0);
    let mut headers = header::HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(
            "Mozilla/5.0 (X11; Linux x86_64; rv:137.0) Gecko/20100101 Firefox/137.0",
        ),
    );
    headers.insert(
        ACCEPT,
        HeaderValue::from_static(
            "image/avif,image/webp,image/png,image/svg+xml,image/*;q=0.8,*/*;q=0.5",
        ),
    );
    headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.5"));
    headers.insert(REFERER, HeaderValue::from_static("http://www.pixiv.net/"));
    headers.insert(CONNECTION, HeaderValue::from_static("keep-alive"));
    headers.insert("Sec-Fetch-Dest", HeaderValue::from_static("image"));
    headers.insert("Sec-Fetch-Mode", HeaderValue::from_static("no-cors"));
    headers.insert("Sec-Fetch-Site", HeaderValue::from_static("cross-site"));
    headers.insert(RANGE, format!("bytes={}-", existing_size).parse()?);

    let response = client.get(url).headers(headers).send().await;
    match response {
        Ok(resp) if resp.status().is_success() => {
            let mut file = OpenOptions::new()
                .append(true)
                .create(true)
                .open(&temp_path)?;
            let mut content = resp.bytes_stream();
            while let Some(chunk) = content.next().await {
                file.write_all(&chunk?)?;
            }
            fs::rename(temp_path, filepath)?;
        }
        _ => {
            eprintln!("[WARN] Failed to download {}, skipping...", url);
        }
    }
    // let mut file = OpenOptions::new()
    //     .append(true)
    //     .create(true)
    //     .open(&temp_path)?;
    // let mut response = client
    //     .get(url)
    //     .headers(headers)
    //     .send()
    //     .await?
    //     .bytes_stream();
    // while let Some(chunk) = response.next().await {
    //     file.write_all(&chunk?)?
    // }
    // fs::rename(temp_path, filepath)?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: pixiv_download <URL>");
        std::process::exit(1);
    }
    let artwork_url = &args[1];
    download_pixiv_artwork(artwork_url).await?;
    Ok(())
}
