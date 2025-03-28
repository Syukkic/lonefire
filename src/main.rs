use anyhow::Result;
use downloader::download_pixiv_artwork;

mod config;
mod downloader;
mod helpers;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: lonefire <URL>\nBTW, lonefire can't download GIF image");
        std::process::exit(1);
    }
    let artwork_url = &args[1];
    download_pixiv_artwork(artwork_url).await?;
    Ok(())
}
