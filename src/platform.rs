/// Detect the current platform in portal format (e.g. "linux-amd64").
pub fn detect_platform() -> String {
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "darwin",
        "windows" => "windows",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "arm" => "armv7",
        other => other,
    };
    format!("{os}-{arch}")
}
