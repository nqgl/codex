//! Linux-only local typesetting. No user files or network are mounted into the worker.
use std::path::Path;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

pub(super) fn render(
    source: &str,
    fg: (u8, u8, u8),
    bg: (u8, u8, u8),
    style: super::MathStyle,
    cell_height: u16,
) -> Option<Vec<u8>> {
    if !cfg!(target_os = "linux") || !super::parser::safe_math(source) {
        return None;
    }
    let directory = tempfile::tempdir().ok()?;
    let (fr, fg, fb) = fg;
    let (br, bg, bb) = bg;
    let (border, math_style, dpi) = match style {
        super::MathStyle::Display => ("4pt", "displaystyle", 360),
        super::MathStyle::Inline => (
            "0.5pt",
            "textstyle",
            (u32::from(cell_height) * 4).clamp(/*min*/ 72, /*max*/ 360),
        ),
    };
    let dpi = dpi.to_string();
    let document = format!(
        r"\documentclass[border={border}]{{standalone}}
\usepackage{{amsmath,amssymb,xcolor}}
\definecolor{{fg}}{{RGB}}{{{fr},{fg},{fb}}}
\definecolor{{bg}}{{RGB}}{{{br},{bg},{bb}}}
\pagecolor{{bg}}
\begin{{document}}\color{{fg}}$\{math_style} {source}$\end{{document}}"
    );
    std::fs::write(directory.path().join("math.tex"), document).ok()?;
    for args in [
        vec![
            "/usr/bin/pdflatex",
            "-no-shell-escape",
            "-interaction=batchmode",
            "-halt-on-error",
            "math.tex",
        ],
        vec![
            "/usr/bin/pdftoppm",
            "-png",
            "-singlefile",
            "-r",
            &dpi,
            "math.pdf",
            "math",
        ],
    ] {
        if !run(directory.path(), &args) {
            return None;
        }
    }
    let path = directory.path().join("math.png");
    if std::fs::metadata(&path).ok()?.len() > 512 * 1024 {
        return None;
    }
    std::fs::read(path).ok()
}

fn run(directory: &Path, args: &[&str]) -> bool {
    let mut command = Command::new("/usr/bin/bwrap");
    command
        .env_clear()
        .args(["--unshare-all", "--die-with-parent", "--new-session"]);
    for path in [
        "/usr",
        "/lib",
        "/lib64",
        "/etc/texmf",
        "/etc/fonts",
        "/var/lib/texmf",
        "/var/cache/fontconfig",
    ] {
        if Path::new(path).exists() {
            command.args(["--ro-bind", path, path]);
        }
    }
    command
        .args([
            "--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp", "--bind",
        ])
        .arg(directory)
        .args([
            "/work",
            "--chdir",
            "/work",
            "--setenv",
            "HOME",
            "/tmp",
            "--setenv",
            "PATH",
            "/usr/bin",
            "--setenv",
            "TEXMFVAR",
            "/tmp",
            "--setenv",
            "openin_any",
            "p",
            "--setenv",
            "openout_any",
            "p",
            "--",
            "/usr/bin/prlimit",
            "--as=536870912",
            "--cpu=3",
            "--fsize=8388608",
            "--",
        ])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
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
