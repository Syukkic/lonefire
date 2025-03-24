use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
pub struct PixivResponse {
    pub illust: HashMap<String, IllustInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IllustInfo {
    pub page_count: u32,
    pub urls: ImageUrls,
    pub user_illusts: Option<HashMap<String, Option<UserIllust>>>,
}

#[derive(Debug, Deserialize)]
pub struct ImageUrls {
    pub original: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UserIllust {
    pub url: Option<String>,
}
