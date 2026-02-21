use std::fs;
use std::process::Command;

const REPO: &str = "IWhitebird/tpdf";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

fn platform_name() -> Result<String, Box<dyn std::error::Error>> {
    let os = if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        return Err("Unsupported OS".into());
    };

    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        return Err("Unsupported architecture".into());
    };

    Ok(format!("tpdf-{os}-{arch}"))
}

fn fetch_latest_tag() -> Result<String, Box<dyn std::error::Error>> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let output = Command::new("curl")
        .args(["-fsSL", &url])
        .output()?;

    if !output.status.success() {
        return Err("Failed to fetch release info from GitHub".into());
    }

    let body = String::from_utf8(output.stdout)?;

    // Extract tag_name from JSON without pulling in serde
    let tag = body
        .split("\"tag_name\"")
        .nth(1)
        .and_then(|s| s.split('"').nth(1))
        .ok_or("Could not parse latest version from GitHub")?;

    Ok(tag.to_string())
}

pub fn self_update() -> Result<(), Box<dyn std::error::Error>> {
    println!("tpdf v{CURRENT_VERSION}");
    println!("Checking for updates...");

    let tag = fetch_latest_tag()?;
    let latest = tag.strip_prefix('v').unwrap_or(&tag);

    if latest == CURRENT_VERSION {
        println!("Already on the latest version.");
        return Ok(());
    }

    println!("New version available: v{latest}");

    let platform = platform_name()?;
    let url = format!("https://github.com/{REPO}/releases/download/{tag}/{platform}.tar.gz");
    let current_exe = std::env::current_exe()?;

    let tmp_dir = tempdir()?;
    let archive = tmp_dir.join("tpdf.tar.gz");

    println!("Downloading {url}...");
    let status = Command::new("curl")
        .args(["-fsSL", &url, "-o"])
        .arg(&archive)
        .status()?;
    if !status.success() {
        return Err("Download failed".into());
    }

    println!("Extracting...");
    let status = Command::new("tar")
        .args(["xzf"])
        .arg(&archive)
        .arg("-C")
        .arg(&tmp_dir)
        .status()?;
    if !status.success() {
        return Err("Extraction failed".into());
    }

    let new_binary = tmp_dir.join("tpdf");

    // Replace the current binary
    // First try a direct rename (same filesystem), fall back to copy
    if fs::rename(&new_binary, &current_exe).is_err() {
        fs::copy(&new_binary, &current_exe)?;
    }

    println!("Updated tpdf to v{latest}!");
    Ok(())
}

/// Create a temporary directory that we clean up ourselves.
fn tempdir() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("tpdf-update-{}", std::process::id()));
    fs::create_dir_all(&dir)?;
    Ok(dir)
}
