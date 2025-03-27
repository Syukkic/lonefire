use futures::{StreamExt, stream};
use regex::Regex;
use reqwest::header::{
    ACCEPT, ACCEPT_LANGUAGE, CONNECTION, HeaderMap, HeaderValue, REFERER, USER_AGENT,
};

use crate::config::MAX_CONCURRENT_DOWNLOADS;

pub fn normalize_image_url(base_url: &str) -> String {
    let re_custom_thumb = Regex::new(r"/c/250x250_80_a2/custom-thumb").unwrap();
    let re_img_master = Regex::new(r"/c/250x250_80_a2/img-master").unwrap();
    let re_square_custom = Regex::new(r"_(?:square|custom)1200\.jpg$").unwrap();

    let base_url = re_custom_thumb.replace(base_url, "/img-original");
    let base_url = re_img_master.replace(&base_url, "/img-original");
    let base_url = re_square_custom.replace(&base_url, ".jpg");

    base_url.into_owned()
}

pub fn generate_image_urls(base_url: &str, page_count: usize) -> Vec<String> {
    (0..page_count)
        .flat_map(|seq| {
            let url_jpg = base_url
                .replace("p0", &format!("p{}", seq))
                .replace(".png", ".jpg");
            let url_png = base_url
                .replace("p0", &format!("p{}", seq))
                .replace(".jpg", ".png");
            vec![url_jpg, url_png]
        })
        .collect()
}

async fn probe_image_url(url: String) -> Option<String> {
    let probe_client = reqwest::Client::new();
    let headers = set_headers();
    match probe_client.head(&url).headers(headers).send().await {
        Ok(response) if response.status().is_success() => Some(url),
        _ => None,
    }
}

pub async fn filter_valid_urls(urls: Vec<String>) -> Vec<String> {
    stream::iter(urls.into_iter().map(probe_image_url))
        .buffer_unordered(MAX_CONCURRENT_DOWNLOADS)
        .filter_map(|url| async { url })
        .collect::<Vec<String>>()
        .await
}

pub fn format_filename(url: &str) -> Option<String> {
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

pub fn set_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
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

    headers
}
