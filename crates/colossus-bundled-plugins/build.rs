//! Track the complete portable core tree and compile its sorted file table.

use std::{env, error::Error, fs, path::Path};

fn collect(root: &Path, directory: &Path, files: &mut Vec<String>) -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed={}", directory.display());
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect(root, &entry.path(), files)?;
        } else if kind.is_file() {
            let relative = entry.path().strip_prefix(root)?.to_owned();
            let relative = relative
                .to_str()
                .ok_or("non-UTF-8 bundled path")?
                .replace('\\', "/");
            files.push(relative);
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
    let mut files = Vec::new();
    collect(&root, &root, &mut files)?;
    files.sort();
    let mut generated =
        String::from("pub static CORE_FILES: &[colossus_plugins::PluginFile<'static>] = &[\n");
    for relative in files {
        let source = root.join(&relative);
        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt as _;
            fs::metadata(&source)?.permissions().mode() & 0o111 != 0
        };
        #[cfg(not(unix))]
        let executable = false;
        println!("cargo:rerun-if-changed={}", source.display());
        generated.push_str(&format!(
            "colossus_plugins::PluginFile {{ path: {relative:?}, bytes: include_bytes!({:?}), executable: {executable} }},\n",
            source.to_str().ok_or("non-UTF-8 source path")?
        ));
    }
    generated.push_str("];\n");
    fs::write(
        Path::new(&env::var("OUT_DIR")?).join("content.rs"),
        generated,
    )?;
    Ok(())
}
