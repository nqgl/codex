//! Local MiTeX and Typst typesetting with a bundled, offline MiTeX package.

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use flate2::read::GzDecoder;

// Published at https://packages.typst.org/preview/mitex-0.2.7.tar.gz (Apache-2.0).
// SHA-256: 0159e214845e49cbdc332d9d572da112dae5ad248072e0a7680d38c8307c2e15.
const MITEX_PACKAGE: &[u8] = include_bytes!("../../assets/math/mitex-0.2.7.tar.gz");

pub(super) fn executable() -> Option<PathBuf> {
    which::which("typst").ok()
}

pub(super) fn render(
    typst: &Path,
    source: &str,
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
    style: super::MathStyle,
    cell_height: u16,
) -> Option<Vec<u8>> {
    if !super::parser::safe_math(source) {
        return None;
    }
    let directory = tempfile::tempdir().ok()?;
    let package_path = directory.path().join("mitex");
    std::fs::create_dir(&package_path).ok()?;
    tar::Archive::new(GzDecoder::new(MITEX_PACKAGE))
        .unpack(package_path)
        .ok()?;

    let (fr, fg, fb) = fg;
    let (br, bg, bb) = bg;
    let (border, size, dpi, equation) = match style {
        super::MathStyle::Display => ("4pt", 18, 240, "mitex"),
        super::MathStyle::Inline => (
            "0.5pt",
            12,
            (u32::from(cell_height) * 4).clamp(/*min*/ 72, /*max*/ 360),
            "mi",
        ),
    };
    let expression = serde_json::to_string(source).ok()?;
    let document = format!(
        "#import \"mitex/lib.typ\": mi, mitex\n\
         #set page(width: auto, height: auto, margin: {border}, fill: rgb({br}, {bg}, {bb}))\n\
         #set text(size: {size}pt, fill: rgb({fr}, {fg}, {fb}))\n\
         #{equation}({expression})\n"
    );
    std::fs::write(directory.path().join("math.typ"), document).ok()?;

    let mut command = Command::new(typst);
    command
        .current_dir(directory.path())
        .env_clear()
        .env("HOME", directory.path())
        .env("XDG_CACHE_HOME", directory.path())
        .args([
            "compile",
            "--ignore-system-fonts",
            "--jobs",
            "1",
            "--format",
            "png",
            "--ppi",
            &dpi.to_string(),
            "--pages",
            "1",
            "--root",
        ])
        .arg(directory.path())
        .args(["math.typ", "math.png"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if !run(&mut command) {
        return None;
    }
    let path = directory.path().join("math.png");
    if std::fs::metadata(&path).ok()?.len() > 512 * 1024 {
        return None;
    }
    std::fs::read(path).ok()
}

fn run(command: &mut Command) -> bool {
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 4);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(/*millis*/ 20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}
