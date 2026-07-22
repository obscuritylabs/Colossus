use super::FILE_DESCRIPTOR_SET;
use prost::Message;
use prost_types::{FileDescriptorSet, MethodDescriptorProto};
use std::collections::BTreeMap;

const PUBLIC_PACKAGE: &str = "colossus.api.v1alpha1";
const FORBIDDEN_INGRESS_FIELDS: &[&str] = &[
    "actor",
    "actor_id",
    "caller",
    "caller_id",
    "credential",
    "credential_value",
    "credentials",
    "environment",
    "executable",
    "path",
    "server_path",
];

fn descriptors() -> FileDescriptorSet {
    FileDescriptorSet::decode(FILE_DESCRIPTOR_SET).expect("generated descriptor set")
}

fn methods() -> BTreeMap<String, MethodDescriptorProto> {
    descriptors()
        .file
        .into_iter()
        .flat_map(|file| {
            file.service.into_iter().flat_map(move |service| {
                let service_name = service.name.unwrap_or_default();
                service.method.into_iter().map(move |method| {
                    (
                        format!(
                            "{service_name}.{}",
                            method.name.as_deref().unwrap_or_default()
                        ),
                        method,
                    )
                })
            })
        })
        .collect()
}

#[test]
fn every_file_is_versioned_in_the_public_package() {
    let descriptors = descriptors();
    let public_files = descriptors
        .file
        .iter()
        .filter(|file| file.package.as_deref() == Some(PUBLIC_PACKAGE))
        .collect::<Vec<_>>();
    assert_eq!(public_files.len(), 6);
    assert!(public_files.iter().all(|file| {
        file.name
            .as_deref()
            .is_some_and(|name| name.starts_with("colossus/api/v1alpha1/"))
    }));
}

#[test]
fn requests_cannot_claim_identity_or_server_execution_inputs() {
    for file in descriptors()
        .file
        .into_iter()
        .filter(|file| file.package.as_deref() == Some(PUBLIC_PACKAGE))
    {
        for message in file.message_type {
            let message_name = message.name.as_deref().unwrap_or_default();
            if !message_name.ends_with("Request") {
                continue;
            }
            for field in message.field {
                let field_name = field.name.as_deref().unwrap_or_default();
                assert!(
                    !FORBIDDEN_INGRESS_FIELDS.contains(&field_name),
                    "{message_name}.{field_name} crosses a forbidden public boundary"
                );
            }
        }
    }
}

#[test]
fn every_enum_has_an_unspecified_zero_value() {
    for file in descriptors()
        .file
        .into_iter()
        .filter(|file| file.package.as_deref() == Some(PUBLIC_PACKAGE))
    {
        for enumeration in file.enum_type {
            let enum_name = enumeration.name.as_deref().unwrap_or_default();
            let zero = enumeration
                .value
                .iter()
                .find(|value| value.number == Some(0))
                .unwrap_or_else(|| panic!("{enum_name} lacks a zero value"));
            assert!(
                zero.name
                    .as_deref()
                    .is_some_and(|name| name.ends_with("_UNSPECIFIED")),
                "{enum_name} zero value must end in _UNSPECIFIED"
            );
        }
    }
}

#[test]
fn durable_run_and_artifact_stream_shapes_are_fixed() {
    let methods = methods();

    let watch = &methods["AgentRunService.WatchRun"];
    assert!(!watch.client_streaming.unwrap_or_default());
    assert!(watch.server_streaming.unwrap_or_default());

    let respond = &methods["AgentRunService.RespondInteraction"];
    assert!(!respond.client_streaming.unwrap_or_default());
    assert!(!respond.server_streaming.unwrap_or_default());

    let upload = &methods["ArtifactService.UploadArtifact"];
    assert!(upload.client_streaming.unwrap_or_default());
    assert!(!upload.server_streaming.unwrap_or_default());

    let download = &methods["ArtifactService.DownloadArtifact"];
    assert!(!download.client_streaming.unwrap_or_default());
    assert!(download.server_streaming.unwrap_or_default());
}
