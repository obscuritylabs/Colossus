use super::types::{Manifest, SavedSetupPackage, invalid, valid_id};
use crate::dto::CommandErrorDto;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{ImageFormat, ImageReader, Limits};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read, Write},
};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

pub(super) const MAX_ARCHIVE_BYTES: usize = 2 * 1024 * 1024;
const MAX_MEMBER_BYTES: u64 = 256 * 1024;
const MAX_EXPANDED_BYTES: usize = 1024 * 1024;
const MAX_MEMBERS: usize = 40;

pub(super) struct PackageSource {
    pub manifest: Manifest,
    pub config_yaml: String,
    pub icons: BTreeMap<String, String>,
    pub ca_pem: Option<String>,
}

/// Read bounded archive members into memory; no archive path reaches the filesystem.
pub(super) fn read(bytes: &[u8]) -> Result<PackageSource, CommandErrorDto> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(invalid("The setup package exceeds 2 MiB."));
    }
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| invalid("Choose a valid ZIP setup package."))?;
    if archive.len() > MAX_MEMBERS {
        return Err(invalid("The setup package has too many files."));
    }
    let mut members = BTreeMap::new();
    let mut names = BTreeSet::new();
    let mut total = 0usize;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|_| invalid("An archive entry is invalid or encrypted."))?;
        let name = entry.name().to_owned();
        let path = name.trim_end_matches('/');
        if path.is_empty()
            || path.len() > 160
            || path.split('/').any(|part| !valid_id(part))
            || !names.insert(path.to_ascii_lowercase())
            || entry
                .unix_mode()
                .is_some_and(|mode| !matches!(mode & 0o170_000, 0 | 0o100_000 | 0o040_000))
        {
            return Err(invalid(
                "Package paths must be unique portable files; links and special files are unsupported.",
            ));
        }
        if entry.is_dir() {
            if !matches!(path, "assets" | "certificates") {
                return Err(invalid("The package contains an unexpected directory."));
            }
            continue;
        }
        if entry.size() > MAX_MEMBER_BYTES {
            return Err(invalid("A package file exceeds 256 KiB."));
        }
        let mut content = Vec::new();
        (&mut entry)
            .take(MAX_MEMBER_BYTES + 1)
            .read_to_end(&mut content)
            .map_err(|_| invalid("A package file could not be read."))?;
        total = total.saturating_add(content.len());
        if content.len() as u64 > MAX_MEMBER_BYTES || total > MAX_EXPANDED_BYTES {
            return Err(invalid(
                "The expanded setup package exceeds its size limit.",
            ));
        }
        members.insert(name, content);
    }
    source_from_members(&members)
}

fn source_from_members(
    members: &BTreeMap<String, Vec<u8>>,
) -> Result<PackageSource, CommandErrorDto> {
    let manifest: Manifest = serde_saphyr::from_slice(
        members
            .get("manifest.yaml")
            .ok_or_else(|| invalid("manifest.yaml is missing."))?,
    )
    .map_err(|_| invalid("The setup manifest is invalid."))?;
    validate_manifest(&manifest)?;
    let config_yaml = String::from_utf8(
        members
            .get("config.yaml")
            .ok_or_else(|| invalid("config.yaml is missing."))?
            .clone(),
    )
    .map_err(|_| invalid("config.yaml must be UTF-8."))?;
    let mut expected = BTreeSet::from(["manifest.yaml".to_owned(), "config.yaml".to_owned()]);
    let mut icons = BTreeMap::new();
    for presentation in manifest.providers.values() {
        for path in [presentation.icon.as_ref(), presentation.dark_icon.as_ref()]
            .into_iter()
            .flatten()
        {
            if !path.starts_with("assets/")
                || !std::path::Path::new(path)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
            {
                return Err(invalid("Icons must reference packaged PNG assets."));
            }
            expected.insert(path.clone());
            if !icons.contains_key(path) {
                let png = members
                    .get(path)
                    .ok_or_else(|| invalid("A referenced icon is missing."))?;
                icons.insert(path.clone(), normalize_png(png)?);
            }
        }
    }
    if icons.values().map(String::len).sum::<usize>() > 256 * 1024 {
        return Err(invalid("Combined provider icons exceed 256 KiB."));
    }
    let ca_pem = manifest
        .ca_bundle
        .as_ref()
        .map(|path| {
            if !path.starts_with("certificates/")
                || !std::path::Path::new(path)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("pem"))
            {
                return Err(invalid(
                    "CA certificates must reference a packaged PEM file.",
                ));
            }
            expected.insert(path.clone());
            let pem = String::from_utf8(
                members
                    .get(path)
                    .ok_or_else(|| invalid("The CA bundle is missing."))?
                    .clone(),
            )
            .map_err(|_| invalid("The CA bundle must be PEM text."))?;
            validate_ca(&pem)?;
            Ok(pem)
        })
        .transpose()?;
    if members.keys().any(|name| !expected.contains(name)) {
        return Err(invalid("The package contains unreferenced files."));
    }
    Ok(PackageSource {
        manifest,
        config_yaml,
        icons,
        ca_pem,
    })
}

fn validate_manifest(manifest: &Manifest) -> Result<(), CommandErrorDto> {
    if manifest.schema_version != 1 {
        return Err(invalid("This setup package version is unsupported."));
    }
    if !valid_id(&manifest.id)
        || manifest.name.trim().is_empty()
        || manifest.name.len() > 96
        || manifest.version.is_empty()
        || manifest.version.len() > 64
        || manifest.description_markdown.len() > 16_384
        || manifest.providers.is_empty()
        || manifest.providers.len() > 16
        || manifest.providers.iter().any(|(id, p)| {
            !valid_id(id)
                || p.display_name.trim().is_empty()
                || p.display_name.len() > 96
                || p.description_markdown.len() > 16_384
        })
    {
        return Err(invalid(
            "Package identity, provider names, or instructions exceed supported limits.",
        ));
    }
    Ok(())
}

pub(super) fn validate_ca(
    pem: &str,
) -> Result<colossus_network::AdditionalRootCertificates, CommandErrorDto> {
    // A setup package contains public trust anchors only, never private key blocks.
    if pem.contains("PRIVATE KEY") || pem.len() as u64 > MAX_MEMBER_BYTES {
        return Err(invalid(
            "The CA bundle must contain only public certificates.",
        ));
    }
    colossus_network::AdditionalRootCertificates::from_pem_bundle(pem.as_bytes())
        .map_err(|_| invalid("The packaged CA certificates are invalid."))
}

fn normalize_png(bytes: &[u8]) -> Result<String, CommandErrorDto> {
    if bytes.len() > 64 * 1024 || bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err(invalid(
            "Provider icons must be PNG files no larger than 64 KiB.",
        ));
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
    let mut limits = Limits::default();
    limits.max_image_width = Some(512);
    limits.max_image_height = Some(512);
    limits.max_alloc = Some(4 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|_| invalid("A provider icon is invalid or exceeds 512 × 512 pixels."))?;
    let mut normalized = Cursor::new(Vec::new());
    decoded
        .write_to(&mut normalized, ImageFormat::Png)
        .map_err(|_| invalid("A provider icon could not be normalized."))?;
    if normalized.get_ref().len() > 64 * 1024 {
        return Err(invalid("The normalized provider icon exceeds 64 KiB."));
    }
    Ok(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(normalized.into_inner())
    ))
}

pub(super) fn write(package: &SavedSetupPackage) -> Result<Vec<u8>, CommandErrorDto> {
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let manifest = serde_saphyr::to_string(&package.manifest)
        .map_err(|_| invalid("The manifest could not be exported."))?;
    let mut files = BTreeMap::from([
        ("manifest.yaml".to_owned(), manifest.into_bytes()),
        (
            "config.yaml".to_owned(),
            package.config_yaml.as_bytes().to_vec(),
        ),
    ]);
    for (path, data) in &package.icons {
        let encoded = data
            .strip_prefix("data:image/png;base64,")
            .ok_or_else(|| invalid("A saved icon is invalid."))?;
        files.insert(
            path.clone(),
            STANDARD
                .decode(encoded)
                .map_err(|_| invalid("A saved icon is invalid."))?,
        );
    }
    if let (Some(path), Some(pem)) = (&package.manifest.ca_bundle, &package.ca_pem) {
        files.insert(path.clone(), pem.as_bytes().to_vec());
    }
    for (name, bytes) in files {
        archive
            .start_file(name, options)
            .map_err(|_| invalid("The setup package could not be exported."))?;
        archive
            .write_all(&bytes)
            .map_err(|_| invalid("The setup package could not be exported."))?;
    }
    archive
        .finish()
        .map(Cursor::into_inner)
        .map_err(|_| invalid("The setup package could not be exported."))
}
