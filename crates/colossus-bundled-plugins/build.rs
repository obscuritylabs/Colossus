//! Embed the core plugin and the canonical documentation tree without a source copy.

use std::{env, error::Error, fs, path::Path};

struct BundledFile {
    path: String,
    bytes: Vec<u8>,
    executable: bool,
}

fn collect(
    root: &Path,
    directory: &Path,
    prefix: &Path,
    files: &mut Vec<BundledFile>,
) -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed={}", directory.display());
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect(root, &entry.path(), prefix, files)?;
        } else if kind.is_file() {
            let source = entry.path();
            let relative = prefix.join(entry.path().strip_prefix(root)?);
            let relative = relative
                .to_str()
                .ok_or("non-UTF-8 bundled path")?
                .replace('\\', "/");
            #[cfg(unix)]
            let executable = {
                use std::os::unix::fs::PermissionsExt as _;
                entry.metadata()?.permissions().mode() & 0o111 != 0
            };
            #[cfg(not(unix))]
            let executable = false;
            println!("cargo:rerun-if-changed={}", source.display());
            files.push(BundledFile {
                path: relative,
                bytes: fs::read(source)?,
                executable,
            });
        } else {
            return Err(format!(
                "bundled content contains a link or special file: {}",
                entry.path().display()
            )
            .into());
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR")?;
    let root = Path::new(&manifest_dir)
        .join("../../bundled-plugins/colossus")
        .canonicalize()?;
    let docs = Path::new(&manifest_dir).join("../../docs").canonicalize()?;
    let mut files = Vec::new();
    collect(&root, &root, Path::new(""), &mut files)?;
    collect(
        &docs,
        &docs,
        Path::new("skills/help/references/docs"),
        &mut files,
    )?;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    for pair in files.windows(2) {
        if pair[0].path == pair[1].path {
            return Err(format!("duplicate bundled path: {}", pair[0].path).into());
        }
    }
    let entries = files
        .iter()
        .map(|file| colossus_plugins::PluginFile {
            path: &file.path,
            bytes: &file.bytes,
            executable: file.executable,
        })
        .collect::<Vec<_>>();
    let artifact = colossus_plugins::build_plugin_artifact_from_files(&entries)?;
    let output = env::var("OUT_DIR")?;
    let output = Path::new(&output);
    fs::write(output.join("core.manifest.json"), artifact.manifest)?;
    fs::write(output.join("core.config.json"), artifact.config)?;
    fs::write(output.join("core.layer.tar.gz"), artifact.layer)?;
    fs::write(output.join("core.digest"), artifact.manifest_digest)?;
    Ok(())
}
