//! Create an isolated, credential-free Echo runtime for local cloud acceptance.
use colossus_home::ColossusHome;
use colossus_runtime::RuntimeConfig;
use std::{io::Write, path::Path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::args_os()
        .nth(1)
        .ok_or("usage: init_runtime ABSOLUTE_PRIVATE_DIR")?;
    let home = ColossusHome::ensure_at(directory)?;
    let root = home.confined_root();
    if root.path().join("config.yaml").exists() {
        return Err("runtime configuration already exists".into());
    }
    let workflows = root.prepare_directory(Path::new("workflows"))?;
    root.prepare_directory(Path::new("api"))?;
    let mut config = RuntimeConfig::offline_template(root.path().join("state.redb"));
    config.workflows.repository = workflows.clone();
    config.workflows.user = workflows;
    let yaml = serde_saphyr::to_string(&config)?;
    let file = root.open_file(Path::new("config.yaml"))?;
    file.file().write_all(yaml.as_bytes())?;
    file.file().sync_all()?;
    root.sync_directory()?;
    println!("Isolated Echo runtime ready at {}", root.path().display());
    Ok(())
}
